# ruxguitar

[![Build status](https://github.com/agourlay/ruxguitar/actions/workflows/ci.yml/badge.svg)](https://github.com/agourlay/ruxguitar/actions/workflows/ci.yml)

A guitar pro tablature player.

The design of the application is described in details in the blog article "[Playing guitar tablatures in Rust](https://agourlay.github.io/ruxguitar-tablature-player/)".

## Limitations

- supports only gp5 files

## Usage

```bash
./ruxguitar --help
Guitar pro tablature player

Usage: ruxguitar [OPTIONS]

Options:
      --sound-font-file <SOUND_FONT_FILE>  Optional path to a sound font file
  -h, --help                               Print help
  -V, --version                            Print version
```

A basic soundfont is embedded in the binary for a plug and play experience, however it is possible to provide a larger soundfont file to get better sound quality.

For instance I like to use `FluidR3_GM.sf2` which is present on most systems and easy to find online ([here](https://musical-artifacts.com/artifacts/738) or [there](https://member.keymusician.com/Member/FluidR3_GM/index.html)).

```bash
./ruxguitar --sound-font-file /usr/share/sounds/sf2/FluidR3_GM.sf2
```

## Installation

### Releases

Using the provided binaries in https://github.com/agourlay/ruxguitar/releases

### Build

Make sure to check the necessary dependencies for your system from the [CI configuration](https://github.com/agourlay/ruxguitar/blob/master/.github/workflows/ci.yml).

## Acknowledgements

This project is heavily inspired by the great [TuxGuitar](https://sourceforge.net/p/tuxguitar/code/HEAD/tree/trunk/) project.