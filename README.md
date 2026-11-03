# jerber

jerber meas Just Gerber or JLC Gerber.

## EDA SUPPORTED
- Altium Designer
- KiCAD
- EasyEDA

## Usage

```sh
# Convert an archive; the result is written next to the input
jerber GERBER.zip
# -> GERBER-AD-L4.zip

# Choose the output path (anything that is not *.zip is written as a folder)
jerber GERBER.zip -o NEW_GERBER.zip

# Convert an already extracted directory
jerber ./gerbers -o ./new_gerbers.zip

# Script-friendly: no per-file report
jerber GERBER.zip -q
```

## License

MIT — see `LICENSE`.

## Special Thanks

- [fuckjlc](https://github.com/acha666/FuckJLC)
