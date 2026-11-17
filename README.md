# jber

[![CI](https://github.com/ref42/jber/actions/workflows/ci.yml/badge.svg)](https://github.com/ref42/jber/actions/workflows/ci.yml)
[![Release](https://github.com/ref42/jber/actions/workflows/release.yml/badge.svg)](https://github.com/ref42/jber/releases/latest)

jber means Just Gerber: it turns an Altium, KiCad or EasyEDA Gerber archive into
a clean manufacturing set that a fab can read.

## EDA SUPPORTED
- Altium Designer
- KiCAD
- EasyEDA

From source:

```sh
cargo install --git https://github.com/ref42/jber
```

From crates.io:

```sh
cargo install jber
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

## License

MIT — see `LICENSE`.
