# random-dir

[![Crates.io Version](https://img.shields.io/crates/v/random-dir)](https://crates.io/crates/random-dir)
[![Docs](https://docs.rs/random-dir/badge.svg)](https://docs.rs/random-dir)
[![dependency status](https://deps.rs/repo/github/igankevich/random-dir/status.svg)](https://deps.rs/repo/github/igankevich/random-dir)

CPIO archive reader/writer library. Supports _New ASCII_, _New CRC_, _Old character_, and _New binary_ formats.


## Introduction

`random-dir` is a library that helps create directories with random contents.
The intended usage is to generate random directory,
feed this directory as an input to some function, and
then compare the output of this function to the expected output.

This crate is particularly useful to test archiver crates like [`kpea`](https://docs.rs/kpea).

## Example

```rust
use random_dir::list_dir_all;
use random_dir::Dir;
use arbtest::arbtest;

fn test_pack_unpack_symmetry() {
    arbtest(|u| {
        let directory: Dir = u.arbitrary()?;
        // Pack the directory to a temporary archive file.
        // Unpack the temporary archive file to `/tmp/unpack-dir`.
        let files1 = list_dir_all(directory.path()).unwrap();
        let files2 = list_dir_all("/tmp/unpack-dir").unwrap();
        assert_eq!(files1, files2);
    });
}
```
