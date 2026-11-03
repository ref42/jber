//! Conversion pipeline: naming, Gerber post-processing, drills, validation.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::Local;
use md5::{Digest, Md5};

use crate::eda::{self, Eda};
use crate::excellon::{self, Plating, Tool};

const GUIDE_NAME: &str = "PCB下单必读.txt";
const GUIDE: &str = "PCB 下单说明\n\n上传本压缩包即可下单。\n参考：\nhttps://prodocs.lceda.cn/cn/pcb/order-order-pcb/index.html";

const DRILL_PTH: &str = "Drill_PTH_Through.DRL";
const DRILL_NPTH: &str = "Drill_NPTH_Through.DRL";

/// Conversion options supplied by the CLI.
#[derive(Clone, Copy, Debug, Default)]
pub struct Settings {
    pub clean: bool,
}

/// One file in the produced set.
#[derive(Clone, Debug)]
pub struct Output {
    pub name: String,
    pub data: Vec<u8>,
    pub plot: bool,
}

/// Everything the CLI needs to report and write.
#[derive(Clone, Debug)]
pub struct Conversion {
    pub outputs: Vec<Output>,
    pub mapping: BTreeMap<String, String>,
    pub warnings: Vec<String>,
    pub eda: Eda,
    pub layers: u32,
    pub renamed: usize,
    pub kept: usize,
    pub drill_sources: usize,
}

/// Convert a flat map of archive entries into a normalized output set.
pub fn convert(
    files: BTreeMap<String, Vec<u8>>,
    settings: &Settings,
) -> Result<Conversion, String> {
    let stamp = Stamp::new();

    let names: Vec<String> = files.keys().cloned().collect();
    let primary = primary_name(&names).ok_or_else(|| "no input files".to_string())?;
    let detected = eda::detect(&files[&primary]);
    let eda =
        detected.ok_or_else(|| format!("cannot determine the EDA of {primary}: {detected:?}"))?;

    let namer = Namer::new(eda, names.iter().map(String::as_str));

    let mut outputs: Vec<Output> = Vec::new();
    let mut mapping: BTreeMap<String, String> = BTreeMap::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut negative_planes: Vec<String> = Vec::new();
    let mut drill_tools: Vec<Tool> = Vec::new();
    let mut drill_sources = 0usize;
    let mut renamed = 0usize;
    let mut kept = 0usize;

    for (name, data) in &files {
        let mapped = namer.rename(name);
        if mapped == *name {
            kept += 1;
        } else {
            renamed += 1;
        }
        mapping.insert(name.clone(), mapped.clone());

        if excellon::is_drill_file(name) {
            drill_sources += 1;
            if excellon::is_blind_buried(name) {
                warnings.push(format!(
                    "skipping {name}: blind/buried vias are not supported"
                ));
                continue;
            }
            let text = String::from_utf8_lossy(data);
            let tools = parse_drill(eda, &text);
            drill_tools.extend(tools);
            continue;
        }

        let converted = match eda::detect(data) {
            Some(_) => {
                let header = stamp.gerber_header();
                if mapped.starts_with("Gerber_InnerLayer")
                    && String::from_utf8_lossy(data)
                        .to_lowercase()
                        .contains("filepolarity,negative")
                {
                    // Collating the notice must not rewrite the plane's
                    // geometry; the ordinary textual normalization below is
                    // still applied.
                    negative_planes.push(mapped.clone());
                }
                let text = format!("{header}{}", String::from_utf8_lossy(data));
                let normalized = normalize_dcodes(&text, eda);
                inject_tracking(&normalized).into_bytes()
            }
            None => data.clone(),
        };

        outputs.push(Output {
            name: mapped.clone(),
            data: converted,
            plot: is_plot(&mapped),
        });
    }

    if !negative_planes.is_empty() {
        warnings.push(format!(
            "negative plane layers kept as-is: {}",
            negative_planes.join(", ")
        ));
    }

    let (layers, layer_warnings) = validate(&outputs)?;
    warnings.extend(layer_warnings);

    if settings.clean {
        outputs.retain(|output| output.plot || output.name == GUIDE_NAME);
    }

    let (pth, npth) = excellon::merge(drill_tools);
    let drill_stamp = stamp.stamp_line();
    if !pth.is_empty() {
        outputs.push(Output {
            name: DRILL_PTH.to_string(),
            data: excellon::render(&pth, Plating::Plated, &drill_stamp).into_bytes(),
            plot: true,
        });
    }
    if !npth.is_empty() {
        outputs.push(Output {
            name: DRILL_NPTH.to_string(),
            data: excellon::render(&npth, Plating::NonPlated, &drill_stamp).into_bytes(),
            plot: true,
        });
    }

    // The order guide is always the final entry, even when the archive already
    // shipped an identical file.
    if let Some(position) = outputs.iter().position(|output| output.name == GUIDE_NAME) {
        let guide = outputs.remove(position);
        outputs.push(guide);
    } else {
        outputs.push(Output {
            name: GUIDE_NAME.to_string(),
            data: GUIDE.as_bytes().to_vec(),
            plot: false,
        });
    }

    Ok(Conversion {
        outputs,
        mapping,
        warnings,
        eda,
        layers,
        renamed,
        kept,
        drill_sources,
    })
}

/// Parse one drill source. Altium and EasyEDA use the Altium dialect; KiCad
/// uses decimal coordinates. EasyEDA archives in the wild appear in both
/// flavours, so fall back to the other parser when the expected one finds no
/// tool definitions at all.
fn parse_drill(eda: Eda, text: &str) -> Vec<Tool> {
    match eda {
        Eda::KiCad => excellon::parse_kicad(text),
        Eda::Altium => excellon::parse_altium(text),
        Eda::EasyEDA => {
            let altium = excellon::parse_altium(text);
            if altium.is_empty() {
                let kicad = excellon::parse_kicad(text);
                if !kicad.is_empty() {
                    return kicad;
                }
            }
            altium
        }
    }
}

// --- Primary file ----------------------------------------------------------

fn primary_name(names: &[String]) -> Option<String> {
    let mut ordered: Vec<&String> = names.iter().collect();
    ordered.sort_by_key(|name| primary_rank(name));
    ordered.first().map(|name| (*name).clone())
}

fn primary_rank(name: &str) -> usize {
    if name.contains("Edge_Cuts") {
        0
    } else if name.contains("F_Cu") {
        1
    } else if name.contains("F_Mask") {
        2
    } else {
        let lower = name.to_lowercase();
        if lower.ends_with(".gto") {
            3
        } else if lower.ends_with(".gtl") {
            4
        } else if lower.ends_with(".gbl") {
            5
        } else if lower.ends_with(".gm13") {
            6
        } else if lower.ends_with(".gm1") {
            7
        } else if lower.ends_with(".gko") {
            8
        } else {
            usize::MAX
        }
    }
}

// --- Naming ----------------------------------------------------------------

struct Namer {
    eda: Eda,
    has_gm_extension: bool,
}

impl Namer {
    fn new<'a>(eda: Eda, names: impl Iterator<Item = &'a str>) -> Self {
        let has_gm_extension = names.into_iter().any(|name| {
            let lower = name.to_lowercase();
            lower.ends_with(".gm1") || lower.ends_with(".gm13")
        });
        Namer {
            eda,
            has_gm_extension,
        }
    }

    fn rename(&self, name: &str) -> String {
        let mapped = match self.eda {
            Eda::KiCad => kicad_name(name),
            Eda::Altium | Eda::EasyEDA => altium_name(name, self.has_gm_extension),
        };
        mapped.unwrap_or_else(|| name.to_string())
    }
}

fn altium_name(name: &str, has_gm_extension: bool) -> Option<String> {
    let lower = name.to_lowercase();

    if lower.ends_with(".gm1") || lower.ends_with(".gm13") {
        return Some("Gerber_BoardOutlineLayer.GKO".to_string());
    }
    if lower.ends_with(".gm") {
        return Some(if has_gm_extension {
            "Gerber_DocumentLayer.GBR".to_string()
        } else {
            "Gerber_BoardOutlineLayer.GKO".to_string()
        });
    }
    if lower.ends_with(".gtl") {
        return Some("Gerber_TopLayer.GTL".to_string());
    }
    if lower.ends_with(".gto") {
        return Some("Gerber_TopSilkscreenLayer.GTO".to_string());
    }
    if lower.ends_with(".gts") {
        return Some("Gerber_TopSolderMaskLayer.GTS".to_string());
    }
    if lower.ends_with(".gtp") {
        return Some("Gerber_TopPasteMaskLayer.GTP".to_string());
    }
    if lower.ends_with(".gbl") {
        return Some("Gerber_BottomLayer.GBL".to_string());
    }
    if lower.ends_with(".gbo") {
        return Some("Gerber_BottomSilkscreenLayer.GBO".to_string());
    }
    if lower.ends_with(".gbs") {
        return Some("Gerber_BottomSolderMaskLayer.GBS".to_string());
    }
    if lower.ends_with(".gbp") {
        return Some("Gerber_BottomPasteMaskLayer.GBP".to_string());
    }
    for index in 1..=6 {
        if lower.ends_with(&format!(".g{index}")) || lower.ends_with(&format!(".gp{index}")) {
            return Some(format!("Gerber_InnerLayer{index}.G{index}"));
        }
    }
    if lower.ends_with(".gd1") {
        return Some("Drill_PTH_Through_GBR.GBR".to_string());
    }
    if lower.ends_with(".gg1") {
        return Some("Drill_PTH_Through_Via_GBR.GBR".to_string());
    }
    if lower.ends_with(".drr") {
        return Some("Drill_MAP_GBR.DRR".to_string());
    }
    if lower.ends_with(".ldp") {
        return Some("Gerber_GBR_JOB.LDP".to_string());
    }
    if lower.ends_with(".txt") && lower.contains("hole") {
        return Some("Drill_Source.DRL".to_string());
    }
    for index in 1..=9 {
        if lower.ends_with(&format!(".tx{index}")) {
            return Some("Drill_Source.DRL".to_string());
        }
    }
    None
}

fn kicad_name(name: &str) -> Option<String> {
    let lower = name.to_lowercase();

    if lower.contains("edge_cuts") {
        return Some("Gerber_BoardOutlineLayer.GKO".to_string());
    }
    if lower.contains("gm") {
        return Some("Gerber_DocumentLayer.GBR".to_string());
    }
    if lower.contains("f_cu") {
        return Some("Gerber_TopLayer.GTL".to_string());
    }
    if lower.contains("f_silkscreen") {
        return Some("Gerber_TopSilkscreenLayer.GTO".to_string());
    }
    if lower.contains("f_mask") {
        return Some("Gerber_TopSolderMaskLayer.GTS".to_string());
    }
    if lower.contains("f_paste") {
        return Some("Gerber_TopPasteMaskLayer.GTP".to_string());
    }
    if lower.contains("b_cu") {
        return Some("Gerber_BottomLayer.GBL".to_string());
    }
    if lower.contains("b_silkscreen") {
        return Some("Gerber_BottomSilkscreenLayer.GBO".to_string());
    }
    if lower.contains("b_mask") {
        return Some("Gerber_BottomSolderMaskLayer.GBS".to_string());
    }
    if lower.contains("b_paste") {
        return Some("Gerber_BottomPasteMaskLayer.GBP".to_string());
    }
    for index in 1..=6 {
        if lower.contains(&format!("in{index}_cu")) {
            return Some(format!("Gerber_InnerLayer{index}.G{index}"));
        }
    }
    if lower.ends_with(".drl") {
        return Some("Drill_Source.DRL".to_string());
    }
    if lower.ends_with("-drl_map")
        || lower.ends_with("_drl_map")
        || lower.ends_with("-drl_map.gbr")
        || lower.ends_with("_drl_map.gbr")
    {
        return Some("Drill_MAP_GBR.GBR".to_string());
    }
    if lower.ends_with(".gbrjob") {
        return Some("Gerber_GBR_JOB.GBRJOB".to_string());
    }
    None
}

fn is_plot(name: &str) -> bool {
    if name == DRILL_PTH || name == DRILL_NPTH {
        return true;
    }
    let Some(rest) = name.strip_prefix("Gerber_") else {
        return false;
    };
    let Some((_, extension)) = rest.rsplit_once('.') else {
        return false;
    };
    matches!(
        extension,
        "GTL"
            | "GTO"
            | "GTS"
            | "GTP"
            | "GBL"
            | "GBO"
            | "GBS"
            | "GBP"
            | "GKO"
            | "G1"
            | "G2"
            | "G3"
            | "G4"
            | "G5"
            | "G6"
    )
}

// --- Validation ------------------------------------------------------------

fn validate(outputs: &[Output]) -> Result<(u32, Vec<String>), String> {
    let has = |prefix: &str| outputs.iter().any(|output| output.name.starts_with(prefix));

    for required in [
        "Gerber_BoardOutlineLayer",
        "Gerber_TopLayer",
        "Gerber_TopSolderMaskLayer",
    ] {
        if !has(required) {
            return Err(format!("missing required layer: {required}"));
        }
    }

    let top = has("Gerber_TopLayer");
    let bottom = has("Gerber_BottomLayer");
    let top_mask = has("Gerber_TopSolderMaskLayer");
    let bottom_mask = has("Gerber_BottomSolderMaskLayer");
    let inner = outputs
        .iter()
        .filter(|output| output.name.starts_with("Gerber_InnerLayer"))
        .count() as u32;

    if top && !top_mask {
        return Err("top copper layer without a top solder mask".to_string());
    }
    if bottom && !bottom_mask {
        return Err("bottom copper layer without a bottom solder mask".to_string());
    }
    if top && inner > 0 && !bottom {
        return Err("inner copper layers without a bottom copper layer".to_string());
    }

    let mut warnings = Vec::new();
    if !has("Gerber_TopSilkscreenLayer") {
        warnings.push("missing top silkscreen layer".to_string());
    }
    if !has("Gerber_TopPasteMaskLayer") {
        warnings.push("missing top paste mask layer".to_string());
    }
    if !has("Gerber_BottomSilkscreenLayer") {
        warnings.push("missing bottom silkscreen layer".to_string());
    }
    if !has("Gerber_BottomPasteMaskLayer") {
        warnings.push("missing bottom paste mask layer".to_string());
    }

    let layers = 1 + u32::from(bottom) + inner;
    Ok((layers, warnings))
}

// --- Run stamp -------------------------------------------------------------

struct Stamp {
    vendor: &'static str,
    version: String,
    timestamp: String,
}

impl Stamp {
    fn new() -> Self {
        let mut rng = Rng::new();
        let vendor = if rng.below(2) == 0 {
            "EasyEDA"
        } else {
            "EasyEDA Pro"
        };
        let major = 2 + rng.below(2);
        let minor = 1 + rng.below(5);
        let patch = 1 + rng.below(42);
        let build = rng.below(3);
        let version = format!("v{major}.{minor}.{patch}.{build}");
        let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        Stamp {
            vendor,
            version,
            timestamp,
        }
    }

    fn gerber_header(&self) -> String {
        format!(
            "G04 {} {}, {}*\nG04 Gerber Generator version 0.3*\n",
            self.vendor, self.version, self.timestamp
        )
    }

    fn stamp_line(&self) -> String {
        format!("{} {}, {}", self.vendor, self.version, self.timestamp)
    }
}

struct Rng {
    state: u64,
}

impl Rng {
    fn new() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos() as u64)
            .unwrap_or(0);
        let pid = u64::from(std::process::id());
        let count = COUNTER.fetch_add(1, Ordering::Relaxed);
        let state =
            nanos ^ (pid << 32) ^ count.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03;
        Rng {
            state: if state == 0 {
                0x9E37_79B9_7F4A_7C15
            } else {
                state
            },
        }
    }

    fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        value ^ (value >> 31)
    }

    fn below(&mut self, limit: u64) -> u64 {
        self.next() % limit
    }
}

// --- Gerber post-processing ------------------------------------------------

fn normalize_dcodes(text: &str, eda: Eda) -> String {
    let lines: Vec<String> = split_lines(text)
        .into_iter()
        .map(|line| {
            let trimmed = line.trim();
            let skip = match eda {
                Eda::Altium | Eda::EasyEDA => {
                    line.contains("G54D")
                        || ((line.starts_with("G01")
                            || line.starts_with("G02")
                            || line.starts_with("G36")
                            || line.starts_with("G37"))
                            && (matches!(trimmed, "D01*" | "D02*" | "D03*")
                                || trimmed.contains('X')
                                || trimmed.contains('Y')))
                }
                Eda::KiCad => line.contains("%ADD") || line.contains("G54D"),
            };
            if !skip && starts_with_dcode(line) {
                format!("G54{line}")
            } else {
                line.to_string()
            }
        })
        .collect();
    lines.join("\n")
}

/// Split on LF, strip a CR belonging to a CRLF pair, and keep a trailing empty
/// element when the text ends with a line ending. Joining the result with `\n`
/// therefore preserves the file's final newline.
fn split_lines(text: &str) -> Vec<&str> {
    text.split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect()
}

fn starts_with_dcode(line: &str) -> bool {
    let Some(rest) = line.strip_prefix('D') else {
        return false;
    };
    let digits = rest
        .bytes()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if !(2..=4).contains(&digits) {
        return false;
    }
    rest.as_bytes().get(digits) == Some(&b'*')
}

fn inject_tracking(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n");
    let normalized = normalized.strip_prefix('\u{feff}').unwrap_or(&normalized);
    if normalized.len() > 30_000_000 {
        return normalized.to_string();
    }

    let lines: Vec<&str> = split_lines(normalized);

    let mut definitions: Vec<(u32, String)> = Vec::new();
    for line in lines.iter().take(200) {
        let trimmed = line.trim();
        if let Some(id) = add_definition(trimmed) {
            definitions.push((id, trimmed.to_string()));
        }
    }
    if definitions.len() < 5 {
        return normalized.to_string();
    }

    let mut ids: Vec<u32> = definitions.iter().map(|(id, _)| *id).collect();
    ids.sort_unstable();
    ids.dedup();
    let id = if ids.len() > 5 {
        ids[5]
    } else {
        *ids.last().unwrap()
    };

    let template = definitions
        .iter()
        .filter(|(definition_id, _)| *definition_id == id)
        .map(|(_, line)| line.clone())
        .min()
        .unwrap();

    let mut shifted: Vec<String> = lines
        .iter()
        .map(|line| shift_definition(line, id))
        .collect();

    let mut hasher = Md5::new();
    hasher.update(shifted.join("\n").as_bytes());
    let digest = hasher.finalize();
    let n = digest[15] % 100;
    let size = format!("0.42{n:02}");

    let stamped = stamp_definition(&template, id, &size);

    let index = tracking_insert_index(&shifted, id);
    shifted.insert(index, stamped);
    shifted.join("\n")
}

fn add_definition(line: &str) -> Option<u32> {
    let rest = line.strip_prefix("%ADD")?;
    let digits = rest
        .bytes()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if !(2..=4).contains(&digits) {
        return None;
    }
    if rest
        .as_bytes()
        .get(digits)
        .is_none_or(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    rest[..digits].parse().ok()
}

fn shift_definition(line: &str, selected: u32) -> String {
    for prefix in ["%ADD", "G54D"] {
        if let Some(rest) = line.strip_prefix(prefix) {
            let digits = rest
                .bytes()
                .take(4)
                .take_while(|byte| byte.is_ascii_digit())
                .count();
            if digits == 0 {
                return line.to_string();
            }
            let Ok(number) = rest[..digits].parse::<u32>() else {
                return line.to_string();
            };
            if number >= selected {
                return format!("{prefix}{}{}", number + 1, &rest[digits..]);
            }
            return line.to_string();
        }
    }
    line.to_string()
}

fn stamp_definition(template: &str, id: u32, size: &str) -> String {
    let bytes = template.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b',' {
            let start = index + 1;
            let mut end = start;
            while end < bytes.len() && (bytes[end].is_ascii_digit() || bytes[end] == b'.') {
                end += 1;
            }
            if end > start {
                let mut stamped = String::new();
                stamped.push_str(&template[..index]);
                stamped.push(',');
                stamped.push_str(size);
                stamped.push_str(&template[end..]);
                return stamped;
            }
        }
        index += 1;
    }
    format!("%ADD{id}C,{size}*%")
}

fn tracking_insert_index(lines: &[String], id: u32) -> usize {
    let previous = format!("%ADD{}", id.saturating_sub(1));
    if let Some(position) = lines.iter().position(|line| line.starts_with(&previous)) {
        return position + 1;
    }
    if let Some(position) = lines.iter().position(|line| line.starts_with("%MO")) {
        for (next, line) in lines.iter().enumerate().skip(position + 1) {
            if line.starts_with("%LP") || line.starts_with('G') {
                return next;
            }
        }
        return position + 1;
    }
    // Append after the last content line but before any trailing empty line
    // that represents the file's final newline.
    let trailing_empty = lines
        .iter()
        .rev()
        .take_while(|line| line.is_empty())
        .count();
    lines.len() - trailing_empty
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn altium_naming_table() {
        assert_eq!(
            altium_name("main.GM", false).as_deref(),
            Some("Gerber_BoardOutlineLayer.GKO")
        );
        assert_eq!(
            altium_name("main.GM", true).as_deref(),
            Some("Gerber_DocumentLayer.GBR")
        );
        assert_eq!(
            altium_name("main.GM1", false).as_deref(),
            Some("Gerber_BoardOutlineLayer.GKO")
        );
        assert_eq!(
            altium_name("main.GM13", false).as_deref(),
            Some("Gerber_BoardOutlineLayer.GKO")
        );
        assert_eq!(
            altium_name("main.GTL", false).as_deref(),
            Some("Gerber_TopLayer.GTL")
        );
        assert_eq!(
            altium_name("main.GP1", false).as_deref(),
            Some("Gerber_InnerLayer1.G1")
        );
        assert_eq!(
            altium_name("main.G2", false).as_deref(),
            Some("Gerber_InnerLayer2.G2")
        );
        assert_eq!(
            altium_name("main.GP6", false).as_deref(),
            Some("Gerber_InnerLayer6.G6")
        );
        assert_eq!(
            altium_name("main-RoundHoles.TXT", false).as_deref(),
            Some("Drill_Source.DRL")
        );
        assert_eq!(
            altium_name("main.TX9", false).as_deref(),
            Some("Drill_Source.DRL")
        );
        assert_eq!(
            altium_name("main.DRR", false).as_deref(),
            Some("Drill_MAP_GBR.DRR")
        );
        assert_eq!(
            altium_name("main.LDP", false).as_deref(),
            Some("Gerber_GBR_JOB.LDP")
        );
        assert_eq!(altium_name("Status Report.Txt", false), None);
        assert_eq!(altium_name("Design Rule Check - main.drc", false), None);
    }

    #[test]
    fn kicad_naming_table() {
        assert_eq!(
            kicad_name("board-Edge_Cuts.gbr").as_deref(),
            Some("Gerber_BoardOutlineLayer.GKO")
        );
        assert_eq!(
            kicad_name("board-F_Cu.gbr").as_deref(),
            Some("Gerber_TopLayer.GTL")
        );
        assert_eq!(
            kicad_name("board-F_Silkscreen.gbr").as_deref(),
            Some("Gerber_TopSilkscreenLayer.GTO")
        );
        assert_eq!(
            kicad_name("board-F_Mask.gbr").as_deref(),
            Some("Gerber_TopSolderMaskLayer.GTS")
        );
        assert_eq!(
            kicad_name("board-B_Cu.gbr").as_deref(),
            Some("Gerber_BottomLayer.GBL")
        );
        assert_eq!(
            kicad_name("board-In1_Cu.gbr").as_deref(),
            Some("Gerber_InnerLayer1.G1")
        );
        assert_eq!(
            kicad_name("board-In6_Cu.gbr").as_deref(),
            Some("Gerber_InnerLayer6.G6")
        );
        assert_eq!(kicad_name("board.drl").as_deref(), Some("Drill_Source.DRL"));
        assert_eq!(
            kicad_name("board-drl_map.gbr").as_deref(),
            Some("Drill_MAP_GBR.GBR")
        );
        assert_eq!(
            kicad_name("board.gbrjob").as_deref(),
            Some("Gerber_GBR_JOB.GBRJOB")
        );
        assert_eq!(kicad_name("board-notes.txt"), None);
    }

    #[test]
    fn altium_gm_uses_archive_wide_gm1_check() {
        let namer = Namer::new(Eda::Altium, ["main.GM", "main.GM1"].into_iter());
        assert_eq!(namer.rename("main.GM"), "Gerber_DocumentLayer.GBR");
        let namer = Namer::new(Eda::Altium, ["main.GM", "main.GTL"].into_iter());
        assert_eq!(namer.rename("main.GM"), "Gerber_BoardOutlineLayer.GKO");
    }

    #[test]
    fn unmatched_names_are_kept() {
        let namer = Namer::new(Eda::Altium, std::iter::empty());
        assert_eq!(namer.rename("notes.txt"), "notes.txt");
    }

    #[test]
    fn dcode_normalization_altium() {
        let input = "D10*\nD1000*\nD1*\nD10000*\nX1Y2D01*\nG54D10*\n%ADD10C,0.1*%\n";
        let expected = "G54D10*\nG54D1000*\nD1*\nD10000*\nX1Y2D01*\nG54D10*\n%ADD10C,0.1*%\n";
        assert_eq!(normalize_dcodes(input, Eda::Altium), expected);
    }

    #[test]
    fn dcode_normalization_easyeda_matches_altium() {
        let input = "D10*\r\nG01*\r\nX1Y2D02*";
        let expected = "G54D10*\nG01*\nX1Y2D02*";
        assert_eq!(normalize_dcodes(input, Eda::EasyEDA), expected);
    }

    #[test]
    fn dcode_normalization_kicad() {
        let input = "D10*\n%ADD10C,0.1*%\nG54D10*\nD999*";
        let expected = "G54D10*\n%ADD10C,0.1*%\nG54D10*\nG54D999*";
        assert_eq!(normalize_dcodes(input, Eda::KiCad), expected);
    }

    #[test]
    fn dcode_normalization_preserves_the_final_newline() {
        assert_eq!(normalize_dcodes("D10*\n", Eda::Altium), "G54D10*\n");
        assert_eq!(normalize_dcodes("D10*", Eda::Altium), "G54D10*");
        assert_eq!(normalize_dcodes("X1Y2D01*\r\n", Eda::Altium), "X1Y2D01*\n");
    }

    #[test]
    fn tracking_aperture_preserves_the_final_newline() {
        let mut input = String::from("%MOMM*%\nG04 test*\n");
        for id in 10..=15 {
            input.push_str(&format!("%ADD{id}C,0.{id}000*%\n"));
        }
        input.push_str("G01*\nM02*\n");
        let output = inject_tracking(&input);
        assert!(output.ends_with('\n'));
    }

    #[test]
    fn tracking_aperture_is_inserted_and_ids_shifted() {
        let mut input = String::from("%MOMM*%\nG04 test*\n");
        for id in 10..=15 {
            input.push_str(&format!("%ADD{id}C,0.{id}000*%\n"));
        }
        input.push_str("G01*\nM02*");

        let output = inject_tracking(&input);
        let lines: Vec<&str> = output.lines().collect();

        assert!(lines.iter().any(|line| *line == "%ADD16C,0.15000*%"));
        assert!(!lines.iter().any(|line| *line == "%ADD15C,0.15000*%"));
        assert!(!lines.iter().any(|line| *line == "%ADD15C,0.42000*%"));

        let stamped = lines
            .iter()
            .find(|line| line.starts_with("%ADD15C,"))
            .expect("stamped definition");
        let value = stamped
            .strip_prefix("%ADD15C,")
            .unwrap()
            .strip_suffix("*%")
            .unwrap();
        assert_eq!(value.len(), 6);
        assert!(value.starts_with("0.42"));
        assert_eq!(output, inject_tracking(&input)); // deterministic
    }

    #[test]
    fn tracking_aperture_skips_small_aperture_tables() {
        let input = "%MOMM*%\n%ADD10C,0.1*%\n%ADD11C,0.2*%\nG01*";
        assert_eq!(inject_tracking(input), input);
    }

    #[test]
    fn tracking_aperture_falls_back_to_mo_line() {
        let mut input = String::from("%MOMM*%\nG04 comment*\n");
        for id in [20, 30, 40, 50, 60, 70] {
            input.push_str(&format!("%ADD{id}C,0.10000*%\n"));
        }
        input.push_str("G01*");
        let output = inject_tracking(&input);
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines[0], "%MOMM*%");
        assert!(lines[1].starts_with("%ADD70C,"));
        assert_eq!(lines[2], "G04 comment*");
    }

    #[test]
    fn easyeda_drills_support_the_kicad_dialect() {
        let text =
            "M48\n;DRILL file {EasyEDA Pro}\nMETRIC\nT1C0.300\n%\nG90\nG05\nT1\nX1.0Y2.0\nM30\n";
        let tools = parse_drill(Eda::EasyEDA, text);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].hits.len(), 1);
    }

    #[test]
    fn easyeda_drills_support_the_altium_dialect() {
        let text = "M48\n;FILE_FORMAT=4:4\nMETRIC,LZ\n;TYPE=PLATED\nT01F00S00C0.3000\n%\nT01\nX00010000Y00020000\nM30\n";
        let tools = parse_drill(Eda::EasyEDA, text);
        assert_eq!(tools.len(), 1);
        assert!((tools[0].hits[0].to.0 - 1.0).abs() < 1e-9);
        assert!((tools[0].hits[0].to.1 - 2.0).abs() < 1e-9);
    }

    #[test]
    fn validation_rejects_missing_outline() {
        let outputs = vec![Output {
            name: "Gerber_TopLayer.GTL".to_string(),
            data: Vec::new(),
            plot: true,
        }];
        assert!(validate(&outputs).is_err());
    }

    #[test]
    fn validation_counts_copper_layers() {
        let names = [
            "Gerber_BoardOutlineLayer.GKO",
            "Gerber_TopLayer.GTL",
            "Gerber_TopSolderMaskLayer.GTS",
            "Gerber_BottomLayer.GBL",
            "Gerber_BottomSolderMaskLayer.GBS",
            "Gerber_InnerLayer1.G1",
            "Gerber_InnerLayer2.G2",
        ];
        let outputs: Vec<Output> = names
            .iter()
            .map(|name| Output {
                name: (*name).to_string(),
                data: Vec::new(),
                plot: true,
            })
            .collect();
        let (layers, warnings) = validate(&outputs).unwrap();
        assert_eq!(layers, 4);
        assert_eq!(warnings.len(), 4);
    }

    #[test]
    fn plot_filter_lists_manufacturing_suffixes() {
        assert!(is_plot("Gerber_TopLayer.GTL"));
        assert!(is_plot("Gerber_InnerLayer3.G3"));
        assert!(is_plot("Gerber_BoardOutlineLayer.GKO"));
        assert!(is_plot("Drill_PTH_Through.DRL"));
        assert!(!is_plot("Gerber_GBR_JOB.LDP"));
        assert!(!is_plot("Drill_MAP_GBR.DRR"));
        assert!(!is_plot("Design Rule Check - main.html"));
    }

    #[test]
    fn primary_rank_prefers_expected_names() {
        assert_eq!(primary_rank("board-Edge_Cuts.gbr"), 0);
        assert_eq!(primary_rank("board-F_Cu.gbr"), 1);
        assert_eq!(primary_rank("main.GTO"), 3);
        assert_eq!(primary_rank("main.GTL"), 4);
        assert_eq!(primary_rank("main.GKO"), 8);
        assert_eq!(primary_rank("notes.txt"), usize::MAX);

        let names = vec![
            "notes.txt".to_string(),
            "main.GTL".to_string(),
            "main.GTO".to_string(),
        ];
        assert_eq!(primary_name(&names).as_deref(), Some("main.GTO"));
    }
}
