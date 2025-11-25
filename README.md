# bmap-rs

The bmap-rs project aims to implement tools related to bmap. The project is written in
rust. The inspiration for it is an existing project that is written in python called 
[bmap-tools](https://salsa.debian.org/debian/bmap-tools). 

Right now the implemented function is copying system images files using bmap, which is
safer and faster than regular cp or dd. That can be used to flash images into block
devices.

## Usage
bmap-rs supports 1 subcommand:
- "copy" - copy a file to another file using a bmap file.
```bash
bmap-rs copy <SOURCE_PATH> <TARGET_PATH>
```

The bmap file is automatically searched in the source directory. The recommendation is 
to name it as the source but with bmap extension.

## License
bmap-rs is licensed under dual Apache-2.0 and MIT licenses.
