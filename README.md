# diffy

[![diffy on crates.io](https://img.shields.io/crates/v/diffy)](https://crates.io/crates/diffy)
[![Documentation (latest release)](https://docs.rs/diffy/badge.svg)](https://docs.rs/diffy/)
[![Documentation (master)](https://img.shields.io/badge/docs-master-59f)](https://bmwill.github.io/diffy/diffy/)
[![License](https://img.shields.io/badge/license-Apache-green.svg)](LICENSE-APACHE)
[![License](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE-MIT)

Tools for finding and manipulating differences between files

## Overview

Diffy provides both a library for Rust applications and command-line tools for working with text differences. It uses Myers' diff algorithm to compute differences between texts.

## Command Line Tools

This crate includes two command-line utilities that mimic the behavior of the standard Unix `diff` and `patch` tools:

### diffy-diff

Compare files line by line and show differences in unified format.

```bash
# Basic usage
diffy-diff file1.txt file2.txt

# With color output (auto-detected by default)
diffy-diff --color=always file1.txt file2.txt

# Ignore case differences
diffy-diff -i file1.txt file2.txt

# Custom context lines
diffy-diff -C 5 file1.txt file2.txt

# Compare directories recursively
diffy-diff -r dir1/ dir2/

# Compare directories with exclusions
diffy-diff -r --exclude="*.log" dir1/ dir2/
```

### diffy-patch

Apply unified diff patches to files.

```bash
# Apply a patch
diffy-patch -i original.txt -p changes.patch

# Create a backup before patching
diffy-patch -i file.txt -p patch.diff --backup

# Apply multi-file patch to directory
diffy-patch -p multi.patch -d /path/to/project/

# Apply patch with path stripping
diffy-patch -p patch.diff -p1 -d ./

# Dry run
diffy-patch -i file.txt -p patch.diff --dry-run
```

### Installation

To install the command-line tools:

```bash
cargo install diffy --features cli
```

Or build from source:

```bash
cargo build --release --bin diffy-diff --bin diffy-patch
```

## Library Usage

See the [documentation](https://docs.rs/diffy/) for library API usage.

## License

This project is available under the terms of either the [Apache 2.0
license](LICENSE-APACHE) or the [MIT license](LICENSE-MIT).
