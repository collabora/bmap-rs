# bmap-rs

## Introduction

`bmap-rs` is a generic tool for copying files using the block map. The idea is that
large files, like raw system image files, can be copied or flashed a lot faster and
more reliably with `bmap-rs` than with traditional tools, like `dd` or `cp`. The
project is written in Rust. The inspiration for it is an existing project that is
written in Python called [bmap-tools](https://salsa.debian.org/debian/bmap-tools).

The goal of rewriting it is to be able to create smaller disk images without Python
dependencies.

Right now the implemented function is copying system images files using bmap, which is
safer and faster than regular cp or dd. It can be used to flash system images into block
devices, but it can also be used for general image flashing purposes.

## Usage
bmap-rs supports 1 subcommand:
- "copy" - copy a file to another file using a bmap file.
```bash
bmap-rs copy <SOURCE_PATH> <TARGET_PATH>
```

The bmap file is automatically searched in the source directory. The recommendation is 
to name it as the source but with bmap extension.

## Concept

This section provides general information about the block map (bmap) necessary
for understanding how `bmap-rs` works. The structure of the section is:

* "Sparse files" - the bmap ideas are based on sparse files, so it is important
  to understand what sparse files are.
* "The block map" - explains what bmap is.
* "Raw images" - the main usage scenario for `bmap-rs` is flashing raw images,
  which this section discusses.
* "Usage scenarios" - describes various possible bmap and `bmap-rs` usage
  scenarios.

### Sparse files

One of the main roles of a filesystem, generally speaking, is to map blocks of
file data to disk sectors. Different file-systems do this mapping differently,
and filesystem performance largely depends on how well the filesystem can do
the mapping. The filesystem block size is usually 4KiB, but may also be 8KiB or
larger.

Obviously, to implement the mapping, the file-system has to maintain some kind
of on-disk index. For any file on the file-system, and any offset within the
file, the index allows you to find the corresponding disk sector, which stores
the file's data. Whenever we write to a file, the filesystem looks up the index
and writes to the corresponding disk sectors. Sometimes the filesystem has to
allocate new disk sectors and update the index (such as when appending data to
the file). The filesystem index is sometimes referred to as the "filesystem
metadata".

What happens if a file area is not mapped to any disk sectors? Is this
possible? The answer is yes. It is possible and these unmapped areas are often
called "holes". And those files which have holes are often called "sparse
files".

All reasonable file-systems like Linux ext[234], btrfs, XFS, or Solaris XFS,
and even Windows' NTFS, support sparse files. Old and less reasonable
filesystems, like FAT, do not support holes.

Reading holes returns zeroes. Writing to a hole causes the filesystem to
allocate disk sectors for the corresponding blocks. Here is how you can create
a 4GiB file with all blocks unmapped, which means that the file consists of a
huge 4GiB hole:

```bash
$ truncate -s 4G image.raw
$ stat image.raw
  File: image.raw
  Size: 4294967296   Blocks: 0     IO Block: 4096   regular file
```

Notice that `image.raw` is a 4GiB file, which occupies 0 blocks on the disk!
So, the entire file's contents are not mapped anywhere. Reading this file would
result in reading 4GiB of zeroes. If you write to the middle of the image.raw
file, you'll end up with 2 holes and a mapped area in the middle.

Therefore:
* Sparse files are files with holes.
* Sparse files help save disk space, because, roughly speaking, holes do not
  occupy disk space.
* A hole is an unmapped area of a file, meaning that it is not mapped anywhere
  on the disk.
* Reading data from a hole returns zeroes.
* Writing data to a hole destroys it by forcing the filesystem to map
  corresponding file areas to disk sectors.
* Filesystems usually operate with blocks, so sizes and offsets of holes are
  aligned to the block boundary.

It is also useful to know that you should work with sparse files carefully. It
is easy to accidentally expand a sparse file, that is, to map all holes to
zero-filled disk areas. For example, `scp` always expands sparse files, the
`tar` and `rsync` tools do the same, by default, unless you use the `--sparse`
option. Compressing and then decompressing a sparse file usually expands it.

There are 2 ioctl's in Linux which allow you to find mapped and unmapped areas:
`FIBMAP` and `FIEMAP`. The former is very old and is probably supported by all
Linux systems, but it is rather limited and requires root privileges. The
latter is a lot more advanced and does not require root privileges, but it is
relatively new (added in Linux kernel, version 2.6.28).

Recent versions of the Linux kernel (starting from 3.1) also support the
`SEEK_HOLE` and `SEEK_DATA` values for the `whence` argument of the standard
`lseek()` system call. They allow positioning to the next hole and the next
mapped area of the file.

Advanced Linux filesystems, in modern kernels, also allow "punching holes",
meaning that it is possible to unmap any aligned area and turn it into a hole.
This is implemented using the `FALLOC_FL_PUNCH_HOLE` `mode` of the
`fallocate()` system call.

### The bmap

The bmap is an XML file, which contains a list of mapped areas, plus some
additional information about the file it was created for, for example:
* SHA256 checksum of the bmap file itself
* SHA256 checksum of the mapped areas
* the original file size
* amount of mapped data

The bmap file is designed to be both easily machine-readable and
human-readable. All the machine-readable information is provided by XML tags.
The human-oriented information is in XML comments, which explain the meaning of
XML tags and provide useful information like amount of mapped data in percent
and in MiB or GiB.

### Raw images

Raw images are the simplest type of system images which may be flashed to the
target block device, block-by-block, without any further processing. Raw images
just "mirror" the target block device: they usually start with the MBR sector.
There is a partition table at the beginning of the image and one or more
partitions containing filesystems, like ext4. Usually, no special tools are
required to flash a raw image to the target block device.

Therefore:
* raw images are distributed in a compressed form, and they are almost as small
  as a tarball (that includes all the data the image would take)
* the bmap file and the `bmap-rs` make it possible to quickly flash the
  compressed raw image to the target block device

And, what is even more important, is that flashing raw images is extremely fast
because you write directly to the block device, and write sequentially.

Another great thing about raw images is that they may be 100% ready-to-go and
all you need to do is to put the image on your device "as-is". You do not have
to know the image format, which partitions and filesystems it contains, etc.
This is simple and robust.

### Usage scenarios

Flashing or copying large images is the main `bmap-rs` use case. The idea is
that if you have a raw image file and its bmap, you can flash it to a device by
writing only the mapped blocks and skipping the unmapped blocks.

What this basically means is that with bmap it is not necessary to try to
minimize the raw image size by making the partitions small, which would require
resizing them. The image can contain huge multi-gigabyte partitions, just like
the target device requires. The image will then be a huge sparse file, with
little mapped data. And because unmapped areas "contain" zeroes, the huge image
will compress extremely well, so the huge image will be very small in
compressed form. It can then be distributed in compressed form, and flashed
very quickly with `bmap-rs` and the bmap file, because `bmap-rs` will decompress
the image on-the-fly and write only mapped areas.

The additional benefit of using bmap for flashing is the checksum verification.
Indeed, the `bmap-rs copy` command verifies the SHA256 checksums while
writing. Integrity of the bmap file itself is also protected by a SHA256
checksum and `bmap-rs` verifies it before starting flashing.

The second usage scenario is reconstructing sparse files Generally speaking, if
you had a sparse file but then expanded it, there is no way to reconstruct it.
In some cases, something like

```bash
$ cp --sparse=always expanded.file reconstructed.file
```

would be enough. However, a file reconstructed this way will not necessarily be
the same as the original sparse file. The original sparse file could have
contained mapped blocks filled with all zeroes (not holes), and, in the
reconstructed file, these blocks will become holes. In some cases, this does
not matter. For example, if you just want to save disk space. However, for raw
images, flashing it does matter, because it is essential to write zero-filled
blocks and not skip them. Indeed, if you do not write the zero-filled block to
corresponding disk sectors which, presumably, contain garbage, you end up with
garbage in those blocks. In other words, when we are talking about flashing raw
images, the difference between zero-filled blocks and holes in the original
image is essential because zero-filled blocks are the required blocks which are
expected to contain zeroes, while holes are just unneeded blocks with no
expectations regarding the contents.

`bmap-rs` may be helpful for reconstructing sparse files properly. Before the
sparse file is expanded, you should generate its bmap. Then you may compress
your file or, otherwise, expand it. Later on, you may reconstruct it using the
`bmap-rs copy` command.

## License

Licensed under either of

 * Apache License, Version 2.0
   ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license
   ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
