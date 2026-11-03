# jerber

jerber meas Just Gerber or JLC Gerber.

## EDA SUPPORTED
- Altium Designer
- KiCAD
- EasyEDA

## Install

```sh
cargo install --path .
```

## Usage

```sh
# Convert an archive; the result is written next to the input
jerber GERBER.zip
# -> GERBER-AD-L4.zip

# Choose the output path (anything that is not *.zip is written as a folder)
jerber GERBER.zip -o NEW_GERBER.zip

# Convert an already extracted directory
jerber ./gerbers -o ./new_gerbers.zip

# Ship only manufacturing layers, drills and the order guide
jerber GERBER.zip --clean

# Re-emit negative inner planes as positive images
jerber GERBER.zip --planes=positive

# Reproducible output (identical bytes on every run)
jerber GERBER.zip --seed 7 -o repeatable.zip

# Script-friendly: no per-file report
jerber GERBER.zip -q
```

Output names follow `<archive>-<AD|Ki|ED>-L<copper layers>.zip`. Existing files
are not overwritten unless `--force` is given.

## Options

| Option | Meaning |
| --- | --- |
| `-o, --out <PATH>` | Output archive (`.zip`) or folder |
| `-q, --quiet` | Summary only |
| `--clean` | Keep only `Gerber_*` layers, merged drills and the guide |
| `--planes <warn\|positive>` | Negative inner planes: warn (default) or convert to positive |
| `--seed <N>` | Deterministic headers/watermark; timestamp from `SOURCE_DATE_EPOCH` |
| `--force` | Allow overwriting an existing output |

## Negative planes

Altium writes internal planes as inverted images flagged
`TF.FilePolarity,Negative`. Fabs that ignore that attribute render the plane as
solid copper with the clearances missing.

jerber never changes that silently: it prints a warning for every inner plane
layer. `--planes=positive` re-emits the plane as a positive image — a dark
rectangle covering the outline, then the original objects drawn clear, with the
polarity attribute rewritten. The rectangle comes from the outline's bounding
box, so check the result in a Gerber viewer before ordering.

Solder-mask layers carry the same attribute in Altium exports; that is normal
(mask openings) and is left untouched.

## Reproducibility

Headers normally carry the current time and a random EasyEDA-style version.
`--seed N` makes those deterministic and pins the timestamp to
`SOURCE_DATE_EPOCH` (default `0`), so repeated runs are byte-identical.

## Notes

- Files that are not plots (DRC reports, `*.apr`, `*.REP`, …) are carried over
  byte-for-byte unless `--clean` is used.
- `cargo test` covers naming tables, D-codes, bounds parsing, watermarking,
  drill parsing, plane conversion and the CLI.

## License

MIT — see `LICENSE`.

## Special Thanks

- [fuckjlc](https://github.com/acha666/FuckJLC)
