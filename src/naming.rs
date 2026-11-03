/* src/naming.rs */

//! Maps vendor layer names onto the canonical `Gerber_*` names.

use std::collections::BTreeMap;

use crate::eda::Eda;

/// Rewrites Altium suffixes; each entry maps a lower-case suffix to the final
/// file name.
const ALTIUM_SUFFIXES: [(&str, &str); 20] = [
    (".gtl", "Gerber_TopLayer.GTL"),
    (".gto", "Gerber_TopSilkscreenLayer.GTO"),
    (".gts", "Gerber_TopSolderMaskLayer.GTS"),
    (".gtp", "Gerber_TopPasteMaskLayer.GTP"),
    (".gbl", "Gerber_BottomLayer.GBL"),
    (".gbo", "Gerber_BottomSilkscreenLayer.GBO"),
    (".gbs", "Gerber_BottomSolderMaskLayer.GBS"),
    (".gbp", "Gerber_BottomPasteMaskLayer.GBP"),
    (".g1", "Gerber_InnerLayer1.G1"),
    (".g2", "Gerber_InnerLayer2.G2"),
    (".g3", "Gerber_InnerLayer3.G3"),
    (".g4", "Gerber_InnerLayer4.G4"),
    (".g5", "Gerber_InnerLayer5.G5"),
    (".g6", "Gerber_InnerLayer6.G6"),
    (".gp1", "Gerber_InnerLayer1.G1"),
    (".gp2", "Gerber_InnerLayer2.G2"),
    (".gp3", "Gerber_InnerLayer3.G3"),
    (".gp4", "Gerber_InnerLayer4.G4"),
    (".gp5", "Gerber_InnerLayer5.G5"),
    (".gp6", "Gerber_InnerLayer6.G6"),
];

/// Nominal names for the drill-adjacent files.
const ALTIUM_DRILL_FILES: [(&str, &str); 2] = [
    (".gd1", "Drill_PTH_Through_GBR.GBR"),
    (".gg1", "Drill_PTH_Through_Via_GBR.GBR"),
];

/// KiCad substrings; checked in order, first hit wins.
const KICAD_SUBSTRINGS: [(&str, &str); 16] = [
    ("edge_cuts", "Gerber_BoardOutlineLayer.GKO"),
    ("gm", "Gerber_DocumentLayer.GBR"),
    ("f_cu", "Gerber_TopLayer.GTL"),
    ("f_silkscreen", "Gerber_TopSilkscreenLayer.GTO"),
    ("f_mask", "Gerber_TopSolderMaskLayer.GTS"),
    ("f_paste", "Gerber_TopPasteMaskLayer.GTP"),
    ("b_cu", "Gerber_BottomLayer.GBL"),
    ("b_silkscreen", "Gerber_BottomSilkscreenLayer.GBO"),
    ("b_mask", "Gerber_BottomSolderMaskLayer.GBS"),
    ("b_paste", "Gerber_BottomPasteMaskLayer.GBP"),
    ("in1_cu", "Gerber_InnerLayer1.G1"),
    ("in2_cu", "Gerber_InnerLayer2.G2"),
    ("in3_cu", "Gerber_InnerLayer3.G3"),
    ("in4_cu", "Gerber_InnerLayer4.G4"),
    ("in5_cu", "Gerber_InnerLayer5.G5"),
    ("in6_cu", "Gerber_InnerLayer6.G6"),
];

/// Builds the complete rename plan for an archive.
pub fn plan(files: &[String], eda: Eda) -> BTreeMap<String, String> {
    // Altium 26 names the profile layer ".GM" when it is the only mechanical
    // layer being plotted. Older exports use ".GM1"/".GM13" for the profile and
    // a bare ".GM" for a drawing layer, so only promote ".GM" when no explicit
    // profile layer is present.
    let explicit_profile = matches!(eda, Eda::Altium | Eda::EasyEDA)
        && files.iter().any(|name| {
            let lower = name.to_ascii_lowercase();
            lower.ends_with(".gm1") || lower.ends_with(".gm13")
        });

    files
        .iter()
        .map(|name| {
            let renamed = match eda {
                Eda::Altium | Eda::EasyEDA => altium(name, explicit_profile),
                Eda::KiCad => kicad(name),
            };
            (
                name.clone(),
                renamed.map(str::to_string).unwrap_or_else(|| name.clone()),
            )
        })
        .collect()
}

fn altium(name: &str, explicit_profile: bool) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();

    if lower.ends_with(".gm1") || lower.ends_with(".gm13") {
        return Some("Gerber_BoardOutlineLayer.GKO");
    }
    if lower.ends_with(".gm") {
        return Some(if explicit_profile {
            "Gerber_DocumentLayer.GBR"
        } else {
            "Gerber_BoardOutlineLayer.GKO"
        });
    }

    for (suffix, renamed) in ALTIUM_SUFFIXES {
        if lower.ends_with(suffix) {
            return Some(renamed);
        }
    }

    for (suffix, renamed) in ALTIUM_DRILL_FILES {
        if lower.ends_with(suffix) {
            return Some(renamed);
        }
    }

    if lower.ends_with(".drr") {
        return Some("Drill_MAP_GBR.DRR");
    }
    if lower.ends_with(".ldp") {
        return Some("Gerber_GBR_JOB.LDP");
    }

    // Excellon sources: "<board>-RoundHoles.TXT", "<board>.TX1", ...
    if lower.ends_with(".txt") && lower.contains("hole") {
        return Some("Drill_Source.DRL");
    }
    if is_numbered_drill(&lower) {
        return Some("Drill_Source.DRL");
    }

    None
}

fn is_numbered_drill(lower: &str) -> bool {
    let bytes = lower.as_bytes();
    if bytes.len() < 4 {
        return false;
    }
    let suffix = &lower[bytes.len() - 4..bytes.len() - 1];
    suffix == ".tx" && matches!(bytes[bytes.len() - 1], b'1'..=b'9')
}

fn kicad(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();

    for (needle, renamed) in KICAD_SUBSTRINGS {
        if lower.contains(needle) {
            return Some(renamed);
        }
    }

    if lower.ends_with(".drl") {
        return Some("Drill_Source.DRL");
    }
    if lower.ends_with("-drl_map")
        || lower.ends_with("_drl_map")
        || lower.ends_with("-drl_map.gbr")
        || lower.ends_with("_drl_map.gbr")
    {
        return Some("Drill_MAP_GBR.GBR");
    }
    if lower.ends_with(".gbrjob") {
        return Some("Gerber_GBR_JOB.GBRJOB");
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan_for(names: &[&str], eda: Eda) -> BTreeMap<String, String> {
        let files: Vec<String> = names.iter().map(|name| name.to_string()).collect();
        plan(&files, eda)
    }

    #[test]
    fn altium_planes_and_bare_profile() {
        let mapping = plan_for(
            &["main.GTL", "main.GP1", "main.GP2", "main.GBL", "main.GM"],
            Eda::Altium,
        );
        assert_eq!(mapping["main.GP1"], "Gerber_InnerLayer1.G1");
        assert_eq!(mapping["main.GP2"], "Gerber_InnerLayer2.G2");
        assert_eq!(mapping["main.GM"], "Gerber_BoardOutlineLayer.GKO");
    }

    #[test]
    fn altium_explicit_profile_keeps_gm_as_document_layer() {
        let mapping = plan_for(&["main.GM", "main.GM13"], Eda::Altium);
        assert_eq!(mapping["main.GM"], "Gerber_DocumentLayer.GBR");
        assert_eq!(mapping["main.GM13"], "Gerber_BoardOutlineLayer.GKO");
    }

    #[test]
    fn altium_drill_sources() {
        let mapping = plan_for(
            &["main-RoundHoles.TXT", "main-SlotHoles.TXT", "main.TX2"],
            Eda::Altium,
        );
        assert_eq!(mapping["main-RoundHoles.TXT"], "Drill_Source.DRL");
        assert_eq!(mapping["main-SlotHoles.TXT"], "Drill_Source.DRL");
        assert_eq!(mapping["main.TX2"], "Drill_Source.DRL");
    }

    #[test]
    fn kicad_names() {
        let mapping = plan_for(
            &[
                "board-Edge_Cuts.gbr",
                "board-F_Cu.gbr",
                "board-In3_Cu.gbr",
                "board-job.gbrjob",
                "board-drl_map.gbr",
            ],
            Eda::KiCad,
        );
        assert_eq!(
            mapping["board-Edge_Cuts.gbr"],
            "Gerber_BoardOutlineLayer.GKO"
        );
        assert_eq!(mapping["board-F_Cu.gbr"], "Gerber_TopLayer.GTL");
        assert_eq!(mapping["board-In3_Cu.gbr"], "Gerber_InnerLayer3.G3");
        assert_eq!(mapping["board-job.gbrjob"], "Gerber_GBR_JOB.GBRJOB");
        assert_eq!(mapping["board-drl_map.gbr"], "Drill_MAP_GBR.GBR");
    }
}
