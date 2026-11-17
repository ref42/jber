# jber

[![CI](https://github.com/ref42/jber/actions/workflows/ci.yml/badge.svg)](https://github.com/ref42/jber/actions/workflows/ci.yml)
[![Release](https://github.com/ref42/jber/actions/workflows/release.yml/badge.svg)](https://github.com/ref42/jber/releases/latest)

jber means Just Gerber: it turns an Altium, KiCad or EasyEDA Gerber archive into
a clean manufacturing set that a fab can read.

## EDA SUPPORTED
- Altium Designer
- KiCAD
- EasyEDA

## Platforms

Every [release](https://github.com/ref42/jber/releases/latest) ships one
self-contained file per platform — no installer, no archive:

| file | platform |
| --- | --- |
| `jber-<version>-windows-x64.exe` | Windows 10/11, x64 |
| `jber-<version>-windows-arm64.exe` | Windows 11 on ARM |
| `jber-<version>-macos-arm64` | macOS, Apple silicon |
| `jber-<version>-macos-x64` | macOS, Intel |
| `jber-<version>-linux-x64` | Linux, x86_64 (statically linked) |
| `jber-<version>-linux-arm64` | Linux, arm64 (statically linked) |

The Linux builds are static, so they run on any distribution regardless of the
glibc version. On macOS and Linux, mark the download executable first:

```sh
chmod +x jber-1.0.0-linux-x64
./jber-1.0.0-linux-x64 --version
```

From source:

```sh
cargo install --git https://github.com/ref42/jber
```

## Usage

```sh
# Convert an archive. The suffix is the EDA detected from the file contents,
# not assumed from the archive name:
jber GERBER.zip
# -> GERBER-<Altium|KiCad|Easy>-L<layers>.zip
#    L<n> = copper layer count

# Choose the output path (anything that is not *.zip is written as a folder):
jber GERBER.zip -o NEW_GERBER.zip
jber GERBER.zip --out NEW_GERBER.zip
jber ./gerbers -o ./new_gerbers.zip

# Keep only manufacturing layers, merged drills and the order guide.
# This is the default: drawings, reports, job files and empty layers are left
# out, because a fab that reads them reports "no usable graphics" instead of
# the board size:
jber GERBER.zip

# ... and keep the drawings, reports and job files as well:
jber GERBER.zip --keep-extra

# Overwrite an existing output:
jber GERBER.zip -o NEW_GERBER.zip --force

# Summary only:
jber GERBER.zip -q
jber GERBER.zip --quiet

# Help and version:
jber --help
jber --version
```

## Development

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
```

CI runs those on Linux, macOS and Windows for every push to `master` and every
pull request. Pushing a `v*` tag builds the binaries for all platforms above and
publishes them as a GitHub release:

```sh
git tag v1.0.0
git push origin v1.0.0
```

## License

MIT — see `LICENSE`.
