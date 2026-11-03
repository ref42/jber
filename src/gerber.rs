/* src/gerber.rs */

//! Gerber content transforms that do not depend on the layer stack.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    Altium,
    KiCad,
}

/// Prefixes aperture-select D-codes with `G54`, which several CAM tools expect
/// before a D-code can be used.
pub fn normalize_dcodes(content: &str, flavor: Flavor) -> String {
    let mut output = String::with_capacity(content.len() + content.len() / 10);

    for (index, line) in content.split('\n').enumerate() {
        if index > 0 {
            output.push('\n');
        }

        match selectable_dcode(line, flavor) {
            Some(token_len) => {
                output.push_str("G54");
                output.push_str(&line[..token_len]);
                output.push_str(&line[token_len..]);
            }
            None => output.push_str(line),
        }
    }

    output
}

fn selectable_dcode(line: &str, flavor: Flavor) -> Option<usize> {
    if skip_line(line, flavor) {
        None
    } else {
        dcode_token_len(line)
    }
}

fn skip_line(line: &str, flavor: Flavor) -> bool {
    match flavor {
        Flavor::KiCad => line.contains("%ADD") || line.contains("G54D"),
        Flavor::Altium => {
            if line.contains("G54D") {
                return true;
            }

            // Gerber geometry lines carry their own operation code; prefixing a
            // bare D01/D02/D03 there would corrupt the plot.
            let geometry = line.starts_with("G01")
                || line.starts_with("G02")
                || line.starts_with("G36")
                || line.starts_with("G37");
            if !geometry {
                return false;
            }

            let trimmed = line.trim();
            trimmed == "D01*"
                || trimmed == "D02*"
                || trimmed == "D03*"
                || trimmed.contains('X')
                || trimmed.contains('Y')
        }
    }
}

/// Length of a leading `D<2..4 digits>*` token, if any.
fn dcode_token_len(line: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    if bytes.first() != Some(&b'D') {
        return None;
    }

    let mut cursor = 1;
    while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
        cursor += 1;
    }

    let digits = cursor - 1;
    if !(2..=4).contains(&digits) {
        return None;
    }
    if bytes.get(cursor) != Some(&b'*') {
        return None;
    }

    Some(cursor + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn altium_selects_are_prefixed() {
        let input = "G01*\nD10*\nX1Y2D02*\n%ADD10C,0.1*%\nD03*";
        let output = normalize_dcodes(input, Flavor::Altium);
        assert_eq!(output, "G01*\nG54D10*\nX1Y2D02*\n%ADD10C,0.1*%\nG54D03*");
    }

    #[test]
    fn kicad_skips_aperture_definitions() {
        let input = "%ADD10C,0.1*%\nD10*\nG54D10*";
        let output = normalize_dcodes(input, Flavor::KiCad);
        assert_eq!(output, "%ADD10C,0.1*%\nG54D10*\nG54D10*");
    }
}
