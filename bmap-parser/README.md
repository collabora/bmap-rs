# bmap-parser

The bmap-parser crate aims to implements the parsing bits for bmap files. The inspiration for it
is an existing project that is written in python called [bmap-tools](https://salsa.debian.org/debian/bmap-tools).

Right now, the implemented function is copying system images files using bmap, which is
safer and faster than regular cp or dd. That can be used to flash images into block
devices.

## Usage
```
use bmap-parser::*;
```
There is a `copy` function that uses bmap file as reference, and a `copy_async` for the
process to work in an asynchronous context.

## License
bmap-rs is licensed under dual Apache-2.0 and MIT licenses.
