/* src/main.rs */

//! `jber` — convert a Gerber archive into a JLCPCB-ready set.

mod eda;
mod excellon;
mod pipeline;

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::eda::Eda;
use crate::pipeline::{Conversion, Output, Settings};
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

const USAGE: &str = "\
jber — convert a Gerber archive into a JLCPCB-ready set

USAGE:
    jber <INPUT.zip|DIR> [-o OUTPUT] [-q|--quiet] [OPTIONS]

OPTIONS:
    -o, --out <PATH>    Output path. A *.zip path is written as an archive, any
                        other path as a folder. Default: <input>-<AD|Ki|ED>-L<layers>.zip
                        next to the input.
    -q, --quiet         Print the summary only
        --clean         Keep only manufacturing layers, drills and the guide
        --force         Overwrite an existing output file
    -h, --help          Show this help
    -V, --version       Show the version
";

fn main() -> ExitCode {
    let options = match parse_args(std::env::args().skip(1)) {
        Ok(Some(options)) => options,
        Ok(None) => return ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("jber: {error}\n\n{USAGE}");
            return ExitCode::FAILURE;
        }
    };

    match run(&options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("jber: {error}");
            ExitCode::FAILURE
        }
    }
}

// --- CLI -------------------------------------------------------------------

struct Options {
    input: PathBuf,
    out: Option<PathBuf>,
    quiet: bool,
    clean: bool,
    force: bool,
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Option<Options>, String> {
    let mut input: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut quiet = false;
    let mut clean = false;
    let mut force = false;
    let mut args = args.peekable();

    while let Some(arg) = args.next() {
        // Long options accept both "--flag value" and "--flag=value".
        let (flag, inline) = if arg.starts_with("--") {
            match arg.split_once('=') {
                Some((flag, value)) => (flag.to_string(), Some(value.to_string())),
                None => (arg.clone(), None),
            }
        } else {
            (arg.clone(), None)
        };

        match flag.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            "-q" | "--quiet" => quiet = true,
            "--clean" => clean = true,
            "--force" => force = true,
            "-o" | "--out" => {
                out = Some(PathBuf::from(take_value(&flag, inline, &mut args)?));
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
    Ok(Some(Options {
        input,
        out,
        quiet,
        clean,
        force,
    }))
}

fn take_value<I: Iterator<Item = String>>(
    flag: &str,
    inline: Option<String>,
    args: &mut std::iter::Peekable<I>,
) -> Result<String, String> {
    if let Some(value) = inline {
        return Ok(value);
    }
    args.next()
        .ok_or_else(|| format!("{flag} requires a value"))
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

    let settings = Settings {
        clean: options.clean,
    };

    let conversion = pipeline::convert(files, &settings)?;

    let out_path = options
        .out
        .clone()
        .unwrap_or_else(|| default_output(&options.input, conversion.eda, conversion.layers));

    if out_path.exists() && !options.force {
        return Err(format!(
            "{} already exists (use --force to overwrite)",
            out_path.display()
        ));
    }

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

fn load(input: &Path) -> Result<BTreeMap<String, Vec<u8>>, String> {
    if input.is_dir() {
        load_dir(input)
    } else if input.is_file() {
        load_zip(input)
    } else {
        Err(format!("no such file or directory: {}", input.display()))
    }
}

fn load_dir(directory: &Path) -> Result<BTreeMap<String, Vec<u8>>, String> {
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
        files.insert(name.to_string(), bytes);
    }

    Ok(files)
}

fn load_zip(path: &Path) -> Result<BTreeMap<String, Vec<u8>>, String> {
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
        files.insert(name.to_string(), bytes);
    }

    Ok(files)
}

/// Editor/OS metadata that never belongs to a Gerber set.
fn is_junk(name: &str) -> bool {
    name == ".DS_Store" || name.starts_with("._") || name == "Thumbs.db"
}

// --- Output ----------------------------------------------------------------

fn write_output(path: &Path, outputs: &[Output]) -> Result<(), String> {
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

fn write_zip(path: &Path, outputs: &[Output]) -> Result<(), String> {
    let file = File::create(path).map_err(|e| format!("cannot create {}: {e}", path.display()))?;
    let mut writer = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    for output in outputs {
        writer
            .start_file(output.name.as_str(), options)
            .map_err(|e| format!("cannot add {}: {e}", output.name))?;
        writer
            .write_all(&output.data)
            .map_err(|e| format!("cannot write {}: {e}", output.name))?;
    }

    writer
        .finish()
        .map_err(|e| format!("cannot finalize {}: {e}", path.display()))?;
    Ok(())
}

fn write_dir(directory: &Path, outputs: &[Output]) -> Result<(), String> {
    fs::create_dir_all(directory)
        .map_err(|e| format!("cannot create {}: {e}", directory.display()))?;

    for output in outputs {
        fs::write(directory.join(&output.name), &output.data)
            .map_err(|e| format!("cannot write {}: {e}", output.name))?;
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
    if options.clean {
        println!("Cleaned   reports and drawings removed");
    }
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

    fn parse(items: &[&str]) -> Options {
        parse_args(args(items).into_iter()).unwrap().unwrap()
    }

    #[test]
    fn parses_input_and_output() {
        let options = parse(&["board.zip", "-o", "out.zip"]);
        assert_eq!(options.input, Path::new("board.zip"));
        assert_eq!(options.out.as_deref(), Some(Path::new("out.zip")));
        assert!(!options.quiet);
    }

    #[test]
    fn parses_processing_flags() {
        let options = parse(&["--clean", "--force", "board.zip"]);
        assert!(options.clean);
        assert!(options.force);
    }

    #[test]
    fn quiet_flag_is_accepted() {
        assert!(parse(&["-q", "board.zip"]).quiet);
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
