/* src/stack.rs */

//! Sanity checks on the converted file set.

/// Layers that must be present for a manufacturable set.
const REQUIRED: [&str; 3] = [
    "Gerber_BoardOutlineLayer",
    "Gerber_TopLayer",
    "Gerber_TopSolderMaskLayer",
];

pub struct Stack {
    pub layers: u32,
    pub warnings: Vec<String>,
}

pub fn check(names: &[String]) -> Result<Stack, Vec<String>> {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    for prefix in REQUIRED {
        if !names.iter().any(|name| name.starts_with(prefix)) {
            errors.push(format!("required layer missing: {prefix}"));
        }
    }

    let has_top = has(names, "Gerber_TopLayer");
    let has_bottom = has(names, "Gerber_BottomLayer");
    let inner = names
        .iter()
        .filter(|name| name.starts_with("Gerber_InnerLayer"))
        .count() as u32;

    if has_top {
        if !has(names, "Gerber_TopSolderMaskLayer")
            && !errors
                .iter()
                .any(|error| error.contains("TopSolderMaskLayer"))
        {
            errors.push("top copper needs Gerber_TopSolderMaskLayer".to_string());
        }
        if !has(names, "Gerber_TopSilkscreenLayer") {
            warnings.push("no top silkscreen layer".to_string());
        }
        if !has(names, "Gerber_TopPasteMaskLayer") {
            warnings.push("no top paste layer (needed for SMD assembly)".to_string());
        }
    }

    if has_bottom {
        if !has(names, "Gerber_BottomSolderMaskLayer") {
            errors.push("bottom copper needs Gerber_BottomSolderMaskLayer".to_string());
        }
        if !has(names, "Gerber_BottomSilkscreenLayer") {
            warnings.push("no bottom silkscreen layer".to_string());
        }
        if !has(names, "Gerber_BottomPasteMaskLayer") {
            warnings.push("no bottom paste layer".to_string());
        }
    }

    if has_top && inner > 0 && !has_bottom {
        errors.push("invalid stackup: top and inner copper require a bottom layer".to_string());
    }

    if !errors.is_empty() {
        return Err(errors);
    }

    Ok(Stack {
        layers: u32::from(has_top) + u32::from(has_bottom) + inner,
        warnings,
    })
}

fn has(names: &[String], prefix: &str) -> bool {
    names.iter().any(|name| name.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn counts_copper_layers() {
        let files = names(&[
            "Gerber_BoardOutlineLayer.GKO",
            "Gerber_TopLayer.GTL",
            "Gerber_TopSolderMaskLayer.GTS",
            "Gerber_InnerLayer1.G1",
            "Gerber_InnerLayer2.G2",
            "Gerber_BottomLayer.GBL",
            "Gerber_BottomSolderMaskLayer.GBS",
        ]);

        let stack = check(&files).expect("stack should validate");
        assert_eq!(stack.layers, 4);
    }

    #[test]
    fn rejects_missing_outline() {
        let files = names(&["Gerber_TopLayer.GTL", "Gerber_TopSolderMaskLayer.GTS"]);
        assert!(check(&files).is_err());
    }
}
