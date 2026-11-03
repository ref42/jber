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

// --- plane polarity --------------------------------------------------------

/// True when the file describes an inverted (negative) plane image.
pub fn is_negative(content: &str) -> bool {
    content
        .to_ascii_lowercase()
        .contains("filepolarity,negative")
}

/// Extent of the artwork in raw coordinate units, read from the plot data.
#[derive(Debug, Clone, Copy)]
pub struct Frame {
    pub x_decimals: u32,
    pub y_decimals: u32,
    pub min_x: i64,
    pub max_x: i64,
    pub min_y: i64,
    pub max_y: i64,
    pub metric: bool,
}

impl Frame {
    fn margin_x(&self) -> i64 {
        margin(self.x_decimals, self.metric)
    }

    fn margin_y(&self) -> i64 {
        margin(self.y_decimals, self.metric)
    }
}

fn margin(decimals: u32, metric: bool) -> i64 {
    let unit = 10_i64.pow(decimals);
    if metric {
        2 * unit
    } else {
        ((2.0 / 25.4) * unit as f64).round() as i64
    }
}

/// Reads the coordinate bounding box, which is enough to cover a plane with a
/// dark rectangle when converting it to positive.
pub fn frame(content: &str) -> Option<Frame> {
    let (x_decimals, y_decimals) = format_decimals(content)?;
    let metric = !content.contains("%MOIN");

    let mut frame = Frame {
        x_decimals,
        y_decimals,
        min_x: i64::MAX,
        max_x: i64::MIN,
        min_y: i64::MAX,
        max_y: i64::MIN,
        metric,
    };

    let mut seen = false;
    for line in content.lines() {
        let mut rest = line.trim();

        if let Some(after) = rest.strip_prefix('X')
            && let Some((value, remainder)) = integer(after, x_decimals)
        {
            frame.min_x = frame.min_x.min(value);
            frame.max_x = frame.max_x.max(value);
            seen = true;
            rest = remainder;
        }

        if let Some(after) = rest.strip_prefix('Y')
            && let Some((value, _)) = integer(after, y_decimals)
        {
            frame.min_y = frame.min_y.min(value);
            frame.max_y = frame.max_y.max(value);
            seen = true;
        }
    }

    if seen { Some(frame) } else { None }
}

/// Re-emits a negative plane as a positive image: a dark rectangle covering the
/// artwork, then every original object drawn as clear.
pub fn to_positive(content: &str, frame: &Frame) -> String {
    let lines: Vec<&str> = content.split('\n').collect();
    let body_start = lines
        .iter()
        .position(|line| starts_body(line.trim()))
        .unwrap_or(lines.len());

    let header = &lines[..body_start];
    let mut body: Vec<&str> = lines[body_start..].to_vec();
    while body
        .last()
        .is_some_and(|line| line.trim().is_empty() || line.trim() == "M02*")
    {
        body.pop();
    }

    let min_x = frame.min_x - frame.margin_x();
    let max_x = frame.max_x + frame.margin_x();
    let min_y = frame.min_y - frame.margin_y();
    let max_y = frame.max_y + frame.margin_y();

    let mut output = String::with_capacity(content.len() + 256);
    for line in header {
        let lower = line.to_ascii_lowercase();
        if lower.contains("filepolarity") {
            output.push_str("G04 #@! TF.FilePolarity,Positive*\n");
            continue;
        }
        if lower.starts_with("%ipneg") {
            continue;
        }
        output.push_str(line);
        output.push('\n');
    }

    output.push_str("%LPD*%\nG36*\n");
    output.push_str(&format!("X{min_x}Y{min_y}D02*\n"));
    output.push_str(&format!("X{max_x}D01*\n"));
    output.push_str(&format!("Y{max_y}D01*\n"));
    output.push_str(&format!("X{min_x}D01*\n"));
    output.push_str(&format!("Y{min_y}D01*\n"));
    output.push_str("D02*\nG37*\n%LPC*%\n");

    for line in &body {
        output.push_str(line);
        output.push('\n');
    }

    output.push_str("%LPD*%\nM02*\n");
    output
}

fn starts_body(line: &str) -> bool {
    line.starts_with("G36")
        || line.starts_with("G01")
        || line.starts_with("G75")
        || line.starts_with("G54D")
        || line.starts_with('X')
        || line.starts_with('Y')
        || is_dcode(line)
}

fn is_dcode(line: &str) -> bool {
    let Some(rest) = line.strip_prefix('D') else {
        return false;
    };
    let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    digits.len() >= 2 && rest[digits.len()..].starts_with('*')
}

fn format_decimals(content: &str) -> Option<(u32, u32)> {
    let rest = &content[content.find("%FSLAX")? + 6..];

    let x_digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    let y_position = rest.find('Y')?;
    let y_digits: String = rest[y_position + 1..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect();

    Some((decimals(&x_digits)?, decimals(&y_digits)?))
}

fn decimals(digits: &str) -> Option<u32> {
    digits.chars().nth(1)?.to_digit(10)
}

fn integer(input: &str, decimals: u32) -> Option<(i64, &str)> {
    let token: String = input
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || *ch == '-' || *ch == '.')
        .collect();

    if token.is_empty() || token == "-" {
        return None;
    }

    let value = if token.contains('.') {
        (token.parse::<f64>().ok()? * 10_f64.powi(decimals as i32)).round() as i64
    } else {
        token.parse::<i64>().ok()?
    };

    Some((value, &input[token.len()..]))
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

    #[test]
    fn reads_the_artwork_bounds() {
        let content = "%FSLAX44Y44*%\n%MOMM*%\nG36*\nX0Y0D02*\nX400000Y300000D01*\nX0Y300000D01*\nG37*\nM02*\n";
        let frame = frame(content).expect("frame");
        assert_eq!(frame.min_x, 0);
        assert_eq!(frame.max_x, 400_000);
        assert_eq!(frame.min_y, 0);
        assert_eq!(frame.max_y, 300_000);
        assert!(frame.metric);
    }

    #[test]
    fn detects_negative_polarity() {
        assert!(is_negative("G04 #@! TF.FilePolarity,Negative*"));
        assert!(!is_negative("G04 #@! TF.FilePolarity,Positive*"));
    }

    #[test]
    fn inverts_a_plane_into_a_positive_image() {
        let content = "%FSLAX44Y44*%\n%MOMM*%\n%ADD10C,0.9000*%\nG04 #@! TF.FilePolarity,Negative*\nG36*\nX100000Y100000D02*\nX200000D01*\nY200000D01*\nX100000D01*\nY100000D01*\nD02*\nG37*\nD10*\nX150000Y150000D02*\nD03*\nM02*\n";
        let frame = frame(content).expect("frame");
        let positive = to_positive(content, &frame);

        assert!(positive.contains("%LPC*%"));
        assert!(positive.contains("G36*"));
        assert!(positive.ends_with("%LPD*%\nM02*\n"));
    }
}
