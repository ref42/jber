# jber

jber meas Just Gerber or JLC Gerber.

## EDA SUPPORTED
- Altium Designer
- KiCAD
- EasyEDA

## Usage

```sh
# Convert an archive. The suffix is the EDA detected from the file contents,
# not assumed from the archive name:
jber GERBER.zip
# -> GERBER-<AD|Ki|ED>-L<layers>.zip
#    AD = Altium, Ki = KiCad, ED = EasyEDA, L<n> = copper layer count

# Choose the output path (anything that is not *.zip is written as a folder):
jber GERBER.zip -o NEW_GERBER.zip
jber GERBER.zip --out NEW_GERBER.zip
jber ./gerbers -o ./new_gerbers.zip

# Keep only manufacturing layers, merged drills and the order guide:
jber GERBER.zip --clean

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

## Special Thanks

- [fuckjlc](https://github.com/acha666/FuckJLC)
