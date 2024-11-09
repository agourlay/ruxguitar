# ruxguitar

[![Build status](https://github.com/agourlay/ruxguitar/actions/workflows/ci.yml/badge.svg)](https://github.com/agourlay/ruxguitar/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/ruxguitar.svg)](https://crates.io/crates/ruxguitar)

A guitar pro tablature player.

The design of the application is described in details in the blog article "[Playing guitar tablatures in Rust](https://agourlay.github.io/ruxguitar-tablature-player/)".

![screenshot](screenshot.png)

## Limitations

- supports only gp5 & gp4 files

## Usage

```bash
./ruxguitar --help
Guitar pro tablature player

Usage: ruxguitar [OPTIONS]

Options:
      --sound-font-file <SOUND_FONT_FILE>  Optional path to a sound font file
      --tab-file-path <TAB_FILE_PATH>      Optional path to tab file to by-pass the file picker
      --no-antialiasing                    Disable antialiasing
  -h, --help                               Print help
  -V, --version                            Print version
```

A basic soundfont is embedded in the binary for a plug and play experience, however it is possible to provide a larger soundfont file to get better sound quality.

For instance I like to use `FluidR3_GM.sf2` which is present on most systems and easy to find online ([here](https://musical-artifacts.com/artifacts/738) or [there](https://member.keymusician.com/Member/FluidR3_GM/index.html)).

```bash
./ruxguitar --sound-font-file /usr/share/sounds/sf2/FluidR3_GM.sf2
```

## FAQ

- **Where can I find guitar pro files?**
  - You can find a lot of guitar pro files on the internet. For instance on [Ultimate Guitar](https://www.ultimate-guitar.com/).

- **Why is the sound quality so bad?**
  - The default soundfont is very basic. You can provide a better soundfont file using the `--sound-font-file` option.

- **Which dependencies are needed to run the application?**
  - Check the necessary dependencies for your system from the [CI configuration](https://github.com/agourlay/ruxguitar/blob/master/.github/workflows/ci.yml).

- **Why is the file picker not opening on Linux?**
  - Install the `XDG Destop Portal` package for your [desktop environment](https://wiki.archlinux.org/title/XDG_Desktop_Portal#List_of_backends_and_interfaces).

- **Why are the strings no rendered on the tablature?**
  - You might need to disable antialiasing using the `--no-antialiasing` option.

- **Does it run on Windows 7 or Windows 8?**
  - Yes, the Windows binary is built with Rust 1.75 which supports Windows 7 and Windows 8.

- **Why is the sound not working on Linux?**
  - You are most likely using `PulseAudio` which is not supported.

## Installation

### Releases

Using the provided binaries in https://github.com/agourlay/ruxguitar/releases

### Crates.io

Using Cargo via [crates.io](https://crates.io/crates/ruxguitar).

```bash
cargo install ruxguitar
```

### Build

Make sure to check the necessary dependencies for your system from the [CI configuration](https://github.com/agourlay/ruxguitar/blob/master/.github/workflows/ci.yml).

## Acknowledgements

This project is heavily inspired by the great [TuxGuitar](https://sourceforge.net/p/tuxguitar/code/HEAD/tree/trunk/) project.