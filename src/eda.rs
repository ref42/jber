//! EDA-tool identification.
//!
//! Detection only looks at the first ten lines of the first 4096 bytes of a
//! file, because every exporter stamps its name into that window.

/// The three supported exporters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Eda {
    Altium,
    KiCad,
    EasyEDA,
}

impl Eda {
    /// Human readable name used in reports.
    pub fn label(self) -> &'static str {
        match self {
            Eda::Altium => "Altium",
            Eda::KiCad => "KiCad",
            Eda::EasyEDA => "EasyEDA",
        }
    }

    /// Simple letter tag used in the default output file name.
    pub fn tag(self) -> &'static str {
        match self {
            Eda::Altium => "Altium",
            Eda::KiCad => "KiCad",
            Eda::EasyEDA => "Easy",
        }
    }
}

/// Inspect a file's first 4096 bytes and report the exporter, if any.
pub(crate) fn detect(bytes: &[u8]) -> Option<Eda> {
    let window = &bytes[..bytes.len().min(4096)];
    let text = String::from_utf8_lossy(window);
    let mut head = String::new();
    for line in text.lines().take(10) {
        head.push_str(line);
        head.push('\n');
    }
    let head = head.to_lowercase();

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_and_tags() {
        assert_eq!(Eda::Altium.label(), "Altium");
        assert_eq!(Eda::KiCad.label(), "KiCad");
        assert_eq!(Eda::EasyEDA.label(), "EasyEDA");
        assert_eq!(Eda::Altium.tag(), "AD");
        assert_eq!(Eda::KiCad.tag(), "Ki");
        assert_eq!(Eda::EasyEDA.tag(), "ED");
    }

    #[test]
    fn detects_altium() {
        let data = b"G04*\nG04 #@! TF.GenerationSoftware,Altium Limited*";
        assert_eq!(detect(data), Some(Eda::Altium));
    }

    #[test]
    fn detects_kicad() {
        let data = b"G04 #@! TF.GenerationSoftware,KiCad,Pcbnew*";
        assert_eq!(detect(data), Some(Eda::KiCad));
    }

    #[test]
    fn detects_easyeda() {
        let data = b"G04 EasyEDA Pro*";
        assert_eq!(detect(data), Some(Eda::EasyEDA));
    }

    #[test]
    fn detection_is_limited_to_ten_lines() {
        let mut data = Vec::new();
        for _ in 0..10 {
            data.extend_from_slice(b"G04 filler*\n");
        }
        data.extend_from_slice(b"G04 Altium*\n");
        assert_eq!(detect(&data), None);
    }

    #[test]
    fn detection_is_limited_to_4096_bytes() {
        let mut data = vec![b'\n'; 5000];
        data.extend_from_slice(b"Altium");
        assert_eq!(detect(&data), None);
    }

    #[test]
    fn recognizes_nothing() {
        assert_eq!(detect(b"M48\nMETRIC,LZ\n%"), None);
    }
}
