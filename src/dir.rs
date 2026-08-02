use std::collections::HashMap;
use std::fs::create_dir_all;
use std::fs::hard_link;
use std::fs::read_link;
use std::fs::File;
use std::io::Error;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::path::MAIN_SEPARATOR_STR;
use std::time::Duration;
use std::time::SystemTime;

use arbitrary::Arbitrary;
use arbitrary::Unstructured;
use normalize_path::NormalizePath;
use tempfile::TempDir;
use walkdir::WalkDir;

/// [`Dir`] configuration.
pub struct DirBuilder {
    printable_names: bool,
    file_types: Vec<FileType>,
}

impl DirBuilder {
    /// Create new directory builder with default parameters.
    pub fn new() -> Self {
        Self {
            #[cfg(not(any(target_os = "macos", windows)))]
            printable_names: false,
            #[cfg(any(target_os = "macos", windows))]
            printable_names: true,
            #[cfg(not(any(target_os = "macos", windows)))]
            file_types: ALL_FILE_TYPES.into(),
            #[cfg(target_os = "macos")]
            file_types: {
                use FileType::*;
                [Regular, Directory, Fifo, Socket, Symlink, HardLink].into()
            },
            #[cfg(target_os = "windows")]
            file_types: {
                use FileType::*;
                [Regular, Directory, Symlink, HardLink].into()
            },
        }
    }

    /// Generate files with printable names, i.e. names consisting only from printable characters.
    ///
    /// Useful to test CLI applications.
    pub fn printable_names(mut self, value: bool) -> Self {
        self.printable_names = value;
        self
    }

    /// Which file types to generate?
    ///
    /// By default any Unix file type can be generated.
    pub fn file_types<I>(mut self, file_types: I) -> Self
    where
        I: IntoIterator<Item = FileType>,
    {
        self.file_types = file_types.into_iter().collect();
        self
    }

    /// Create a temprary directory with random contents.
    pub fn create(self, u: &mut Unstructured<'_>) -> arbitrary::Result<Dir> {
        use FileType::*;
        #[cfg(unix)]
        let random_path = |u: &mut Unstructured<'_>| -> arbitrary::Result<PathBuf> {
            let path = if self.printable_names {
                let len: usize = u.int_in_range(1..=10)?;
                let mut string = String::with_capacity(len);
                for _ in 0..len {
                    string.push(u.int_in_range(b'a'..=b'z')? as char);
                }
                std::ffi::CString::new(string).unwrap()
            } else {
                u.arbitrary()?
            };
            use std::os::unix::ffi::OsStringExt;
            let path = std::ffi::OsString::from_vec(path.into_bytes());
            let path: PathBuf = path.into();
            Ok(path)
        };
        #[cfg(not(unix))]
        let random_path = |u: &mut Unstructured<'_>| -> arbitrary::Result<PathBuf> {
            let len: usize = u.int_in_range(1..=10)?;
            let mut string = String::with_capacity(len);
            loop {
                string.clear();
                for _ in 0..len {
                    string.push(u.int_in_range(b'a'..=b'z')? as char);
                }
                if !is_reserved_file_name(&string) {
                    break;
                }
            }
            Ok(string.into())
        };
        let dir = tempfile::Builder::new().rand_bytes(32).tempdir().unwrap();
        let mut files = Vec::new();
        let num_files: usize = u.int_in_range(0..=10)?;
        for _ in 0..num_files {
            let path = random_path(u)?;
            if path.as_os_str().is_empty() {
                // do not allow empty paths
                continue;
            }
            let path = match path.strip_prefix(MAIN_SEPARATOR_STR) {
                Ok(path) => path,
                Err(_) => path.as_path(),
            };
            let path = dir.path().join(path).normalize();
            if path.is_dir() || files.contains(&path) {
                // the path aliased some existing directory
                continue;
            }
            create_dir_all(path.parent().unwrap()).unwrap();
            let mut kind: FileType = *u.choose(&self.file_types[..])?;
            if matches!(kind, FileType::HardLink | FileType::Symlink) && files.is_empty() {
                kind = Regular;
            }
            let t = {
                let t = SystemTime::now() + Duration::from_secs(60 * 60 * 24);
                let dt = t.duration_since(SystemTime::UNIX_EPOCH).unwrap();
                SystemTime::UNIX_EPOCH
                    + Duration::new(
                        u.int_in_range(0..=dt.as_secs())?,
                        u.int_in_range(0..=999_999_999)?,
                    )
            };
            match kind {
                Regular => {
                    let contents: Vec<u8> = u.arbitrary()?;
                    let mut file = File::create(&path).unwrap();
                    file.write_all(&contents).unwrap();
                    #[cfg(unix)]
                    {
                        use std::fs::Permissions;
                        use std::os::unix::fs::PermissionsExt;
                        let mode = u.int_in_range(0..=0o777)? | 0o400;
                        file.set_permissions(Permissions::from_mode(mode)).unwrap();
                    }
                    file.set_modified(t).unwrap();
                }
                #[cfg(unix)]
                Directory => {
                    use std::os::unix::fs::DirBuilderExt;
                    let mode = u.int_in_range(0..=0o777)? | 0o500;
                    std::fs::DirBuilder::new()
                        .mode(mode)
                        .recursive(true)
                        .create(&path)
                        .unwrap();
                    let path = crate::path_to_c_string(path.clone()).unwrap();
                    crate::set_file_modified_time(&path, t).unwrap();
                }
                #[cfg(not(unix))]
                Directory => {
                    std::fs::DirBuilder::new()
                        .recursive(true)
                        .create(&path)
                        .unwrap();
                }
                #[cfg(unix)]
                Fifo => {
                    let mode = u.int_in_range(0..=0o777)? | 0o400;
                    let path = crate::path_to_c_string(path.clone()).unwrap();
                    crate::mkfifo(&path, mode).unwrap();
                    crate::set_file_modified_time(&path, t).unwrap();
                }
                #[cfg(unix)]
                Socket => {
                    use std::os::unix::net::UnixDatagram;
                    UnixDatagram::bind(&path).unwrap();
                    let path = crate::path_to_c_string(path.clone()).unwrap();
                    crate::set_file_modified_time(&path, t).unwrap();
                }
                #[cfg(unix)]
                BlockDevice => {
                    // dev loop
                    let dev = libc::makedev(7, 0);
                    let mode = u.int_in_range(0o400..=0o777)?;
                    let path = crate::path_to_c_string(path.clone()).unwrap();
                    crate::mknod(&path, mode, dev).unwrap();
                    crate::set_file_modified_time(&path, t).unwrap();
                }
                #[cfg(unix)]
                CharDevice => {
                    let dev = arbitrary_char_dev();
                    let mode = u.int_in_range(0o400..=0o777)?;
                    let path = crate::path_to_c_string(path.clone()).unwrap();
                    crate::mknod(&path, mode, dev).unwrap();
                    crate::set_file_modified_time(&path, t).unwrap();
                }
                #[cfg(unix)]
                Symlink => {
                    use std::os::unix::fs::symlink;
                    let original = u.choose(&files[..]).unwrap();
                    symlink(original, &path).unwrap();
                }
                #[cfg(windows)]
                Symlink => {
                    use std::os::windows::fs::symlink_file;
                    let original = u.choose(&files[..]).unwrap();
                    symlink_file(original, &path).unwrap();
                }
                #[cfg(all(not(unix), not(windows)))]
                Symlink => panic!("Unsupported file type: {kind:?}"),
                HardLink => {
                    let original = u.choose(&files[..]).unwrap();
                    assert!(
                        hard_link(original, &path).is_ok(),
                        "original = `{}`, path = `{}`",
                        original.display(),
                        path.display()
                    );
                }
                #[cfg(not(unix))]
                Socket | Fifo | CharDevice | BlockDevice => {
                    panic!("Unsupported file type: {kind:?}")
                }
            }
            if kind != FileType::Directory {
                files.push(path.clone());
            }
        }
        Ok(Dir { dir })
    }
}

impl Default for DirBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Directory with randomly generated contents.
///
/// Automatically Deleted on drop.
pub struct Dir {
    dir: TempDir,
}

impl Dir {
    /// Get directory path.
    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Transform into inner representation.
    pub fn into_inner(self) -> TempDir {
        self.dir
    }
}

impl<'a> Arbitrary<'a> for Dir {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        DirBuilder::new().create(u)
    }
}

/// File type.
#[derive(Arbitrary, Debug, PartialEq, Eq, Clone, Copy)]
pub enum FileType {
    /// Regular file.
    Regular,
    /// A directory.
    Directory,
    /// Named pipe.
    Fifo,
    /// UNIX socket.
    Socket,
    /// Block device.
    BlockDevice,
    /// Character device.
    CharDevice,
    /// Symbolic link.
    Symlink,
    /// Hard link.
    HardLink,
}

/// All file types supported by the platform.
pub const ALL_FILE_TYPES: [FileType; 8] = {
    use FileType::*;
    [
        Regular,
        Directory,
        Fifo,
        Socket,
        BlockDevice,
        CharDevice,
        Symlink,
        HardLink,
    ]
};

/// Recursively list specified directory.
///
/// This function always returns the same entries in the same order for the same directory.
/// It also remaps inodes to make listings of the two directories conataining the same files
/// consistent.
///
/// The intended usage is to compare the contents (files and metadata) of the two directories.
pub fn list_dir_all<P: AsRef<Path>>(dir: P) -> Result<Vec<FileInfo>, Error> {
    let dir = dir.as_ref();
    let mut files = Vec::new();
    for entry in WalkDir::new(dir).into_iter() {
        let entry = entry?;
        if entry.path() == dir {
            continue;
        }
        let metadata = entry.path().symlink_metadata()?;
        let contents = if metadata.is_file() {
            std::fs::read(entry.path()).unwrap()
        } else if metadata.is_symlink() {
            let target = read_link(entry.path()).unwrap();
            target.as_os_str().as_encoded_bytes().to_vec()
        } else {
            Vec::new()
        };
        let path = entry.path().strip_prefix(dir).map_err(Error::other)?;
        let metadata: Metadata = (&metadata).try_into()?;
        files.push(FileInfo {
            path: path.to_path_buf(),
            metadata,
            contents,
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    // remap inodes
    use std::collections::hash_map::Entry::*;
    let mut inodes = HashMap::new();
    let mut next_inode = 0;
    for file in files.iter_mut() {
        let old = file.metadata.ino;
        let inode = match inodes.entry(old) {
            Vacant(v) => {
                let inode = next_inode;
                v.insert(next_inode);
                next_inode += 1;
                inode
            }
            Occupied(o) => *o.get(),
        };
        file.metadata.ino = inode;
    }
    Ok(files)
}

/// File's path, metadata and contents.
#[derive(PartialEq, Eq, Debug, Clone)]
pub struct FileInfo {
    /// Path.
    pub path: PathBuf,
    /// Metadata.
    pub metadata: Metadata,
    /// File contents.
    pub contents: Vec<u8>,
}

/// File's metadata.
#[derive(PartialEq, Eq, Clone, Debug)]
pub struct Metadata {
    /// Containing device number.
    pub dev: u64,
    /// Inode.
    pub ino: u64,
    /// File mode.
    pub mode: u32,
    /// Owner's user id.
    pub uid: u32,
    /// Owner's group id.
    pub gid: u32,
    /// No. of hard links.
    pub nlink: u32,
    /// Device number of the file itself.
    pub rdev: u64,
    /// Last modification time.
    pub mtime: u64,
    /// File size in bytes.
    pub file_size: u64,
}

impl TryFrom<&std::fs::Metadata> for Metadata {
    type Error = Error;

    #[cfg(unix)]
    fn try_from(other: &std::fs::Metadata) -> Result<Self, Error> {
        use std::os::unix::fs::MetadataExt;
        Ok(Self {
            dev: other.dev(),
            ino: other.ino(),
            mode: other.mode(),
            uid: other.uid(),
            gid: other.gid(),
            nlink: other.nlink() as u32,
            rdev: other.rdev(),
            mtime: other.mtime() as u64,
            file_size: other.size(),
        })
    }

    #[cfg(not(unix))]
    fn try_from(other: &std::fs::Metadata) -> Result<Self, Error> {
        Ok(Self {
            dev: 0,
            ino: 0,
            mode: 0,
            uid: 0,
            gid: 0,
            nlink: 1,
            rdev: 0,
            mtime: other
                .modified()?
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_secs(),
            file_size: other.len(),
        })
    }
}

#[cfg(target_os = "linux")]
fn arbitrary_char_dev() -> libc::dev_t {
    // /dev/null
    libc::makedev(1, 3)
}

#[cfg(target_os = "macos")]
fn arbitrary_char_dev() -> libc::dev_t {
    // /dev/null
    libc::makedev(3, 2)
}

#[cfg(windows)]
fn is_reserved_file_name(file_name: &str) -> bool {
    let file_name = file_name.to_lowercase();
    for prefix in RESERVED_FILE_PREFIXES.iter() {
        if file_name.starts_with(prefix) {
            return true;
        }
    }
    false
}

#[cfg(all(not(unix), not(windows)))]
fn is_reserved_file_name(_file_name: &str) -> bool {
    false
}

// https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file
#[cfg(windows)]
const RESERVED_FILE_PREFIXES: &[&str] = &["con", "prn", "aux", "nul", "com", "lpt"];
