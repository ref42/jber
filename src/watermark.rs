/* src/watermark.rs */

//! Embeds the tracking aperture: one existing aperture is re-dimensioned with a
//! content-derived size and re-inserted, so the file carries a per-content
//! marker without changing the artwork.

use std::collections::BTreeMap;

use md5::{Digest, Md5};

pub fn embed(content: &str, foreign: bool) -> String {
    let mut text = content.replace("\r\n", "\n");
    if let Some(stripped) = text.strip_prefix('\u{feff}') {
        text = stripped.to_string();
    }
    if text.len() > 30_000_000 {
        return text;
    }

    let Some((template, target, original)) = pick_aperture(&text) else {
        return text;
    };

    let shifted = shift_apertures(&text, original);
    let size = fingerprint_size(&shifted, foreign);
    let stamped = stamped_aperture(&template, target, original, &size);

    place_aperture(&shifted, &stamped, target)
}

/// Chooses the aperture used as the marker: the sixth distinct id (or the
/// highest one when there are fewer), with the lexicographically smallest
/// definition as its template.
fn pick_aperture(text: &str) -> Option<(String, u32, u32)> {
    let mut definitions = 0usize;
    let mut groups: BTreeMap<u32, Vec<String>> = BTreeMap::new();

    for line in text.lines().take(200) {
        let line = line.trim();
        if let Some(id) = aperture_id(line) {
            definitions += 1;
            groups.entry(id).or_default().push(line.to_string());
        }
    }

    if definitions < 5 {
        return None;
    }

    let ids: Vec<u32> = groups.keys().copied().collect();
    let original = if ids.len() > 5 { ids[5] } else { *ids.last()? };
    let template = groups.get(&original)?.iter().min()?.clone();

    Some((template, original, original))
}

fn aperture_id(line: &str) -> Option<u32> {
    let rest = line.strip_prefix("%ADD")?;

    let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    if !(2..=4).contains(&digits.len()) {
        return None;
    }
    if rest.as_bytes().get(digits.len())?.is_ascii_digit() {
        return None;
    }

    digits.parse().ok()
}

/// Bumps every aperture id at or above the marker id, freeing the marker slot.
fn shift_apertures(text: &str, original: u32) -> String {
    let mut output = String::with_capacity(text.len());

    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            output.push('\n');
        }
        output.push_str(&shift_line(line, original));
    }

    output
}

fn shift_line(line: &str, original: u32) -> String {
    let (prefix, rest) = if let Some(rest) = line.strip_prefix("%ADD") {
        ("%ADD", rest)
    } else if let Some(rest) = line.strip_prefix("G54D") {
        ("G54D", rest)
    } else {
        return line.to_string();
    };

    let digits: String = rest
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .take(4)
        .collect();
    if digits.len() < 2 {
        return line.to_string();
    }

    let Ok(number) = digits.parse::<u32>() else {
        return line.to_string();
    };
    if number < original {
        return line.to_string();
    }

    format!("{prefix}{}{}", number + 1, &rest[digits.len()..])
}

fn fingerprint_size(text: &str, foreign: bool) -> String {
    let mut hasher = Md5::new();
    if foreign {
        hasher.update(b"494d");
    }
    hasher.update(text.as_bytes());

    let digest = hasher.finalize();
    let last = digest[digest.len() - 1];
    format!("0.42{:02}", u32::from(last) % 100)
}

fn stamped_aperture(template: &str, target: u32, original: u32, size: &str) -> String {
    let re_sized = replace_first_dimension(template, size)
        .unwrap_or_else(|| format!("%ADD{target}C,{size}*%"));

    re_sized.replace(&format!("ADD{original}"), &format!("ADD{target}"))
}

fn replace_first_dimension(template: &str, size: &str) -> Option<String> {
    let bytes = template.as_bytes();

    for (index, byte) in bytes.iter().enumerate() {
        if *byte != b',' {
            continue;
        }

        let start = index + 1;
        let mut end = start;
        while end < bytes.len() && (bytes[end].is_ascii_digit() || bytes[end] == b'.') {
            end += 1;
        }
        if end == start {
            continue;
        }

        let mut result = String::new();
        result.push_str(&template[..index]);
        result.push(',');
        result.push_str(size);
        result.push_str(&template[end..]);
        return Some(result);
    }

    None
}

/// Inserts the stamped aperture after the aperture that precedes it, or right
/// before the first operation once the headers are done.
fn place_aperture(text: &str, aperture: &str, target: u32) -> String {
    let anchor = format!("%ADD{}", target.saturating_sub(1));
    let mut lines: Vec<&str> = text.split('\n').collect();

    if let Some(position) = lines.iter().position(|line| line.starts_with(&anchor)) {
        lines.insert(position + 1, aperture);
        return lines.join("\n");
    }

    let mut saw_units = false;
    for (index, line) in lines.iter().enumerate() {
        if line.starts_with("%MO") {
            saw_units = true;
            continue;
        }
        if saw_units && (line.starts_with("%LP") || line.starts_with('G')) {
            lines.insert(index, aperture);
            return lines.join("\n");
        }
    }

    lines.push(aperture);
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gerber(apertures: &[&str], body: &str) -> String {
        let mut text = String::from("%FSLAX44Y44*%\n%MOMM*%\n%LPD*%\n");
        for aperture in apertures {
            text.push_str(aperture);
            text.push('\n');
        }
        text.push_str(body);
        text
    }

    #[test]
    fn leaves_small_files_alone() {
        let text = gerber(&["%ADD10C,0.1000*%", "%ADD11C,0.2000*%"], "D10*\nX0Y0D02*");
        assert_eq!(embed(&text, false), text);
    }

    #[test]
    fn stamps_and_renumbers_large_files() {
        let text = gerber(
            &[
                "%ADD10C,0.1000*%",
                "%ADD11C,0.2000*%",
                "%ADD12C,0.3000*%",
                "%ADD13C,0.4000*%",
                "%ADD14C,0.5000*%",
                "%ADD15C,0.6000*%",
                "%ADD16C,0.7000*%",
            ],
            "D10*\nX0Y0D02*\nM02*",
        );

        let stamped = embed(&text, false);
        assert_eq!(
            stamped.matches("%ADD").count(),
            text.matches("%ADD").count() + 1
        );
        assert!(stamped.contains("0.42"));
    }

    #[test]
    fn is_deterministic_for_the_same_content() {
        let text = gerber(
            &[
                "%ADD10C,0.1000*%",
                "%ADD11C,0.2000*%",
                "%ADD12C,0.3000*%",
                "%ADD13C,0.4000*%",
                "%ADD14C,0.5000*%",
                "%ADD15C,0.6000*%",
            ],
            "D10*\nM02*",
        );

        assert_eq!(embed(&text, false), embed(&text, false));
    }
}
