/* src/eda.rs */

//! Source-EDA detection and the ordering used to pick the "primary" file.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Eda {
    Altium,
    KiCad,
    EasyEDA,
}

impl Eda {
    pub fn label(self) -> &'static str {
        match self {
            Eda::Altium => "Altium",
            Eda::KiCad => "KiCad",
            Eda::EasyEDA => "EasyEDA",
        }
    }

    /// Suffix used in the converted archive name.
    pub fn tag(self) -> &'static str {
        match self {
            Eda::Altium => "AD",
            Eda::KiCad => "Ki",
            Eda::EasyEDA => "ED",
        }
    }
}

/// Recognizes the generating tool from a file's leading lines. Altium wins over
/// KiCad wins over EasyEDA when several markers happen to be present.
pub fn detect(head: &str) -> Option<Eda> {
    let head = head.to_ascii_lowercase();

    if head.contains("altium") {
        Some(Eda::Altium)
    } else if head.contains("kicad") {
        Some(Eda::KiCad)
    } else if head.contains("easyeda") {
        Some(Eda::EasyEDA)
    } else {
        None
    }
}

/// KiCad names that identify the outline/copper files.
const KICAD_MARKERS: [&str; 3] = ["Edge_Cuts", "F_Cu", "F_Mask"];

/// Extensions that Altium and most other tools use for plot files, in the
/// order they are preferred when guessing which file is representative.
const PLOT_EXTENSIONS: [&str; 6] = ["gto", "gtl", "gbl", "gm13", "gm1", "gko"];

/// Returns the file names ordered so that the most representative Gerber comes
/// first. The sort is stable, so equal-ranked files keep their archive order.
pub fn order_for_primary(names: &[String]) -> Vec<String> {
    let mut ordered = names.to_vec();
    ordered.sort_by_key(|name| rank(name));
    ordered
}

fn rank(name: &str) -> usize {
    for (index, marker) in KICAD_MARKERS.iter().enumerate() {
        if name.contains(marker) {
            return index;
        }
    }

    let lower = name.to_ascii_lowercase();
    for (index, extension) in PLOT_EXTENSIONS.iter().enumerate() {
        if lower.ends_with(&format!(".{extension}")) {
            return KICAD_MARKERS.len() + index;
        }
    }

    usize::MAX
}
