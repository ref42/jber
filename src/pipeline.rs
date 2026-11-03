/* src/pipeline.rs */

//! Conversion pipeline: name mapping, header stamping, D-code normalization,
//! drill merging and stack validation.

use std::collections::BTreeMap;

use crate::eda::{self, Eda};
use crate::{banner, excellon, gerber, naming, stack, watermark};

pub const GUIDE_NAME: &str = "PCB下单必读.txt";

pub struct Conversion {
    pub outputs: Vec<(String, String)>,
    pub mapping: BTreeMap<String, String>,
    pub warnings: Vec<String>,
    pub eda: Eda,
    pub layers: u32,
    pub renamed: usize,
    pub kept: usize,
    pub drill_sources: usize,
}

pub fn convert(files: BTreeMap<String, String>) -> Result<Conversion, String> {
    let names: Vec<String> = files.keys().cloned().collect();

    let mut software: BTreeMap<&str, Option<Eda>> = BTreeMap::new();
    for name in &names {
        let head: String = files[name]
            .split('\n')
            .take(10)
            .collect::<Vec<_>>()
            .join("\n");
        software.insert(name, eda::detect(&head));
    }

    let ordered = eda::order_for_primary(&names);
    let primary = ordered
        .first()
        .ok_or_else(|| "empty file list".to_string())?;
    let primary_eda = software[primary.as_str()].ok_or_else(|| {
        format!(
            "unsupported source: \"{primary}\" was identified as \"{}\" (Altium, KiCad and EasyEDA only)",
            software[primary.as_str()].map(Eda::label).unwrap_or("None")
        )
    })?;

    let mapping = naming::plan(&names, primary_eda);
    let header = banner::gerber_header();

    let mut outputs: Vec<(String, String)> = Vec::new();
    let mut drill_sources: Vec<(String, String)> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    for name in &names {
        let renamed = mapping.get(name).cloned().unwrap_or_else(|| name.clone());

        if excellon::is_drill_file(name) {
            drill_sources.push((name.clone(), files[name].replace("\r\n", "\n")));
            continue;
        }

        // Not every file in an archive is a plot: reports and drawings are
        // carried through unchanged.
        let mut content = files[name].replace("\r\n", "\n");
        if let Some(source) = software[name.as_str()] {
            content = format!("{header}{content}");
            content = gerber::normalize_dcodes(&content, flavor(source));
            content = watermark::embed(&content, false);
        }

        outputs.push((renamed, content));
    }

    let merged = excellon::merge(&drill_sources);
    warnings.extend(merged.warnings);
    if let Some(plated) = merged.plated {
        outputs.push(("Drill_PTH_Through.DRL".to_string(), plated));
    }
    if let Some(non_plated) = merged.non_plated {
        outputs.push(("Drill_NPTH_Through.DRL".to_string(), non_plated));
    }

    let final_names: Vec<String> = outputs.iter().map(|(name, _)| name.clone()).collect();
    let stack = stack::check(&final_names)
        .map_err(|errors| format!("gerber validation failed:\n  - {}", errors.join("\n  - ")))?;
    warnings.extend(stack.warnings);

    if !outputs.iter().any(|(name, _)| name == GUIDE_NAME) {
        outputs.push((GUIDE_NAME.to_string(), banner::order_guide().to_string()));
    }

    let (renamed, kept) = names
        .iter()
        .filter(|name| !excellon::is_drill_file(name))
        .fold((0, 0), |(renamed, kept), name| {
            if mapping.get(name.as_str()).map(String::as_str) == Some(name.as_str()) {
                (renamed, kept + 1)
            } else {
                (renamed + 1, kept)
            }
        });

    Ok(Conversion {
        outputs,
        mapping,
        warnings,
        eda: primary_eda,
        layers: stack.layers,
        renamed,
        kept,
        drill_sources: drill_sources.len(),
    })
}

fn flavor(source: Eda) -> gerber::Flavor {
    match source {
        Eda::KiCad => gerber::Flavor::KiCad,
        Eda::Altium | Eda::EasyEDA => gerber::Flavor::Altium,
    }
}
