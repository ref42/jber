/* src/main.rs */

//! `jerber` — convert a Gerber archive into a JLCPCB-ready set.

mod banner;
mod eda;
mod excellon;
mod gerber;
mod naming;
mod pipeline;
mod stack;
mod watermark;

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::eda::Eda;
use crate::pipeline::Conversion;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

const USAGE: &str = "\
jerber — convert a Gerber archive into a JLCPCB-ready set

USAGE:
    jerber <INPUT.zip|DIR> [-o OUTPUT] [-q|--quiet]

OPTIONS:
    -o, --out <PATH>   Output path. A *.zip path is written as an archive, any
                       other path as a folder. Default: <input>-<AD|Ki|ED>-L<layers>.zip
                       next to the input.
    -q, --quiet        Print the summary only
    -h, --help         Show this help
    -V, --version      Show the version
";

fn main() -> ExitCode {
    let options = match parse_args(std::env::args().skip(1)) {
        Ok(Some(options)) => options,
        Ok(None) => return ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("jerber: {error}\n\n{USAGE}");
            return ExitCode::FAILURE;
        }
    };

    match run(&options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("jerber: {error}");
            ExitCode::FAILURE
        }
    }
}

// --- CLI -------------------------------------------------------------------

struct Options {
    input: PathBuf,
    out: Option<PathBuf>,
    quiet: bool,
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Option<Options>, String> {
    let mut input: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut quiet = false;
    let mut args = args.peekable();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("jerber {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            "-q" | "--quiet" => quiet = true,
            "-o" | "--out" => {
                let value = args
                    .next()
                    .ok_or_else(|| format!("{arg} requires a path"))?;
                out = Some(PathBuf::from(value));
            }
            other if other.starts_with('-') && other.len() > 1 => {
                return Err(format!("unknown option: {other}"));
            }
            other => {
                if input.is_some() {
                    return Err(format!("unexpected extra argument: {other}"));
                }
                input = Some(PathBuf::from(other));
            }
        }
    }

    let input = input.ok_or_else(|| "missing input archive".to_string())?;
    Ok(Some(Options { input, out, quiet }))
}

// --- Driver ----------------------------------------------------------------

fn run(options: &Options) -> Result<(), String> {
    let files = load(&options.input)?;
    if files.is_empty() {
        return Err(format!(
            "{} contains no Gerber files",
            options.input.display()
        ));
    }

    let conversion = pipeline::convert(files)?;

    let out_path = options
        .out
        .clone()
        .unwrap_or_else(|| default_output(&options.input, conversion.eda, conversion.layers));

    write_output(&out_path, &conversion.outputs)?;

    report(options, &conversion, &out_path);
    Ok(())
}

fn default_output(input: &Path, eda: Eda, layers: u32) -> PathBuf {
    let base = input
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "gerber".to_string());
    let directory = match input.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    directory.join(format!("{base}-{}-L{layers}.zip", eda.tag()))
}

// --- Input -----------------------------------------------------------------

fn load(input: &Path) -> Result<BTreeMap<String, String>, String> {
    if input.is_dir() {
        load_dir(input)
    } else if input.is_file() {
        load_zip(input)
    } else {
        Err(format!("no such file or directory: {}", input.display()))
    }
}

fn load_dir(directory: &Path) -> Result<BTreeMap<String, String>, String> {
    let mut files = BTreeMap::new();

    for entry in
        fs::read_dir(directory).map_err(|e| format!("cannot read {}: {e}", directory.display()))?
    {
        let path = entry.map_err(|e| e.to_string())?.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if is_junk(name) {
            continue;
        }
        let bytes = fs::read(&path).map_err(|e| format!("cannot read {name}: {e}"))?;
        files.insert(
            name.to_string(),
            String::from_utf8_lossy(&bytes).into_owned(),
        );
    }

    Ok(files)
}

fn load_zip(path: &Path) -> Result<BTreeMap<String, String>, String> {
    let file = File::open(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    let mut archive = ZipArchive::new(file)
        .map_err(|e| format!("{} is not a readable zip: {e}", path.display()))?;

    let mut files = BTreeMap::new();

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| format!("cannot read zip entry #{index}: {e}"))?;

        let entry_name = entry
            .name()
            .map_err(|e| format!("cannot read zip entry #{index}: {e}"))?
            .into_owned();

        if entry.is_dir() || entry_name.starts_with("__MACOSX/") {
            continue;
        }

        let Some(name) = Path::new(&entry_name).file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if is_junk(name) {
            continue;
        }

        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|e| format!("cannot read {name}: {e}"))?;
        files.insert(
            name.to_string(),
            String::from_utf8_lossy(&bytes).into_owned(),
        );
    }

    Ok(files)
}

/// Editor/OS metadata that never belongs to a Gerber set.
fn is_junk(name: &str) -> bool {
    name == ".DS_Store" || name.starts_with("._") || name == "Thumbs.db"
}

// --- Output ----------------------------------------------------------------

fn write_output(path: &Path, outputs: &[(String, String)]) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }

    let is_zip = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"));

    if is_zip {
        write_zip(path, outputs)
    } else {
        write_dir(path, outputs)
    }
}

fn write_zip(path: &Path, outputs: &[(String, String)]) -> Result<(), String> {
    let file = File::create(path).map_err(|e| format!("cannot create {}: {e}", path.display()))?;
    let mut writer = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    for (name, content) in outputs {
        writer
            .start_file(name.as_str(), options)
            .map_err(|e| format!("cannot add {name}: {e}"))?;
        writer
            .write_all(content.as_bytes())
            .map_err(|e| format!("cannot write {name}: {e}"))?;
    }

    writer
        .finish()
        .map_err(|e| format!("cannot finalize {}: {e}", path.display()))?;
    Ok(())
}

fn write_dir(directory: &Path, outputs: &[(String, String)]) -> Result<(), String> {
    fs::create_dir_all(directory)
        .map_err(|e| format!("cannot create {}: {e}", directory.display()))?;

    for (name, content) in outputs {
        fs::write(directory.join(name), content)
            .map_err(|e| format!("cannot write {name}: {e}"))?;
    }
    Ok(())
}

// --- Report ----------------------------------------------------------------

fn report(options: &Options, conversion: &Conversion, out_path: &Path) {
    let entries = conversion.outputs.len();
    let layer_word = if conversion.layers == 1 { "" } else { "s" };
    let source_word = if conversion.drill_sources == 1 {
        ""
    } else {
        "s"
    };

    println!("Input     {}", options.input.display());
    println!(
        "Design    {} · {} copper layer{layer_word}",
        conversion.eda.label(),
        conversion.layers
    );
    println!(
        "Renamed   {} ({} kept unchanged)",
        conversion.renamed, conversion.kept
    );
    println!(
        "Drills    {} source{source_word} merged into PTH/NPTH",
        conversion.drill_sources
    );
    println!("Added     order guide");
    println!();

    if !options.quiet {
        for (original, renamed) in &conversion.mapping {
            if renamed == original || excellon::is_drill_file(original) {
                continue;
            }
            println!("  {original:<30} -> {renamed}");
        }
        println!();
    }

    for warning in &conversion.warnings {
        println!("warning: {warning}");
    }

    println!("Output    {} ({entries} entries)", out_path.display());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn parses_input_and_output() {
        let options = parse_args(args(&["board.zip", "-o", "out.zip"]).into_iter())
            .unwrap()
            .unwrap();
        assert_eq!(options.input, Path::new("board.zip"));
        assert_eq!(options.out.as_deref(), Some(Path::new("out.zip")));
        assert!(!options.quiet);
    }

    #[test]
    fn quiet_flag_is_accepted() {
        let options = parse_args(args(&["-q", "board.zip"]).into_iter())
            .unwrap()
            .unwrap();
        assert!(options.quiet);
    }

    #[test]
    fn missing_input_is_rejected() {
        assert!(parse_args(args(&[]).into_iter()).is_err());
        assert!(parse_args(args(&["-q"]).into_iter()).is_err());
    }

    #[test]
    fn default_output_matches_the_web_naming() {
        let path = default_output(Path::new("gerbers/board.zip"), Eda::Altium, 4);
        assert_eq!(path, Path::new("gerbers/board-AD-L4.zip"));

        let path = default_output(Path::new("gerbers/board.zip"), Eda::KiCad, 2);
        assert_eq!(path, Path::new("gerbers/board-Ki-L2.zip"));
    }

    #[test]
    fn metadata_files_are_skipped() {
        assert!(is_junk(".DS_Store"));
        assert!(is_junk("._main.GTL"));
        assert!(!is_junk("main.GTL"));
    }
}
