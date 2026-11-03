# jerber

jerber meas Just Gerber or JLC Gerber.

## EDA SUPPORTED
- Altium Designer
- KiCAD
- EasyEDA

## Usage

```sh
# Convert an archive. The suffix is the EDA detected from the file contents,
# not assumed from the archive name:
jerber GERBER.zip
# -> GERBER-<AD|Ki|ED>-L<layers>.zip
#    AD = Altium, Ki = KiCad, ED = EasyEDA, L<n> = copper layer count

# Choose the output path (anything that is not *.zip is written as a folder):
jerber GERBER.zip -o NEW_GERBER.zip
jerber GERBER.zip --out NEW_GERBER.zip
jerber ./gerbers -o ./new_gerbers.zip

# Keep only manufacturing layers, merged drills and the order guide:
jerber GERBER.zip --clean

# Negative inner planes: warn (default) or re-emit them as positive images:
jerber GERBER.zip --planes warn
jerber GERBER.zip --planes positive

# Reproducible output (headers + watermark); timestamp comes from
# SOURCE_DATE_EPOCH when set:
jerber GERBER.zip --seed 7 -o repeatable.zip

# Overwrite an existing output:
jerber GERBER.zip -o NEW_GERBER.zip --force

# Summary only:
jerber GERBER.zip -q
jerber GERBER.zip --quiet

# Help and version:
jerber --help
jerber --version
```

## License

MIT — see `LICENSE`.

## Special Thanks

- [fuckjlc](https://github.com/acha666/FuckJLC)
