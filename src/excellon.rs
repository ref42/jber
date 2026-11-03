/* src/excellon.rs */

//! Excellon handling: parse drilling/routing data, merge it per hole size and
//! emit the two files (plated / non-plated) that fabs expect.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use crate::banner;

const INCH: f64 = 25.4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unit {
    Metric,
    Inch,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Hit {
    Hole { x: f64, y: f64 },
    Slot { x1: f64, y1: f64, x2: f64, y2: f64 },
}

struct Tool {
    diameter: f64,
    plated: bool,
    hits: Vec<Hit>,
}

pub struct Merge {
    pub plated: Option<String>,
    pub non_plated: Option<String>,
    pub warnings: Vec<String>,
}

/// Whether the archive entry is an Excellon drill source.
pub fn is_drill_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();

    lower.ends_with(".drl")
        || (lower.ends_with(".txt") && (lower.contains("hole") || lower.contains("drill")))
        || is_numbered(&lower)
}

/// `.TX1`..`.TX6` are blind/buried via layers; JLC only takes through holes.
fn is_numbered(lower: &str) -> bool {
    let bytes = lower.as_bytes();
    bytes.len() >= 4
        && &lower[bytes.len() - 4..bytes.len() - 1] == ".tx"
        && matches!(bytes[bytes.len() - 1], b'1'..=b'6')
}

fn is_through(name: &str) -> bool {
    !is_numbered(&name.to_ascii_lowercase())
}

pub fn merge(sources: &[(String, String)], stamp: &mut banner::Stamp) -> Merge {
    let mut plated: BTreeMap<u64, Tool> = BTreeMap::new();
    let mut non_plated: BTreeMap<u64, Tool> = BTreeMap::new();
    let mut warnings = Vec::new();

    for (name, content) in sources {
        if !is_through(name) {
            warnings.push(format!(
                "skipping {name}: blind/buried vias are not supported, only through holes"
            ));
            continue;
        }

        let tools = if content.to_ascii_lowercase().contains("kicad") {
            parse_kicad(content)
        } else {
            parse_altium(content)
        };

        for tool in tools {
            let key = (tool.diameter * 100_000.0) as u64;
            let bucket = if tool.plated {
                &mut plated
            } else {
                &mut non_plated
            };

            match bucket.entry(key) {
                Entry::Occupied(mut entry) => entry.get_mut().hits.extend(tool.hits),
                Entry::Vacant(entry) => {
                    entry.insert(tool);
                }
            }
        }
    }

    Merge {
        plated: emit(&plated, true, stamp),
        non_plated: emit(&non_plated, false, stamp),
        warnings,
    }
}

fn emit(tools: &BTreeMap<u64, Tool>, plated: bool, stamp: &mut banner::Stamp) -> Option<String> {
    if tools.is_empty() {
        return None;
    }

    let (hole_type, layer) = if plated {
        ("PLATED", "PTH_Through")
    } else {
        ("NON_PLATED", "NPTH_Through")
    };

    let mut output = stamp.drill_header(hole_type, layer);
    output.push_str("M48\nMETRIC,LZ,0000.00000\n");

    for (index, tool) in tools.values().enumerate() {
        let number = index + 1;
        output.push_str(&format!(
            ";Hole size {number} = {:.5} METRIC\n",
            tool.diameter
        ));
        output.push_str(&format!("T{number:02}C{:.5}\n", tool.diameter));
    }

    output.push_str("%\nG05\nG90\n");

    for (index, tool) in tools.values().enumerate() {
        output.push_str(&format!("T{:02}\n", index + 1));
        for hit in &tool.hits {
            match hit {
                Hit::Hole { x, y } => output.push_str(&format!("X{x:.5}Y{y:.5}\n")),
                Hit::Slot { x1, y1, x2, y2 } => {
                    output.push_str(&format!("X{x1:.5}Y{y1:.5}G85X{x2:.5}Y{y2:.5}\n"));
                }
            }
        }
    }

    output.push_str("M30\n");
    Some(output)
}

// --- token helpers ---------------------------------------------------------

fn take_digits(input: &str) -> Option<(u64, &str)> {
    let digits: String = input.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    Some((digits.parse().ok()?, &input[digits.len()..]))
}

fn take_number(input: &str) -> Option<(f64, &str)> {
    let token: String = input
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || *ch == '.')
        .collect();
    if token.is_empty() {
        return None;
    }
    Some((token.parse().ok()?, &input[token.len()..]))
}

fn take_coordinate(input: &str) -> Option<(&str, &str)> {
    let length = input
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || *ch == '.' || *ch == '-')
        .count();
    if length == 0 {
        return None;
    }
    Some((&input[..length], &input[length..]))
}

fn tool_select(line: &str) -> Option<u32> {
    let rest = line.strip_prefix('T')?;
    let (number, rest) = take_digits(rest)?;
    if rest.is_empty() {
        Some(number as u32)
    } else {
        None
    }
}

/// Parses `X<num>Y<num>` requiring the whole line to be consumed.
fn coordinate(line: &str) -> Option<(Option<&str>, Option<&str>)> {
    let mut rest = line;
    let mut x = None;
    let mut y = None;

    if let Some(after) = rest.strip_prefix('X') {
        let (value, remainder) = take_coordinate(after)?;
        x = Some(value);
        rest = remainder;
    }
    if let Some(after) = rest.strip_prefix('Y') {
        let (value, remainder) = take_coordinate(after)?;
        y = Some(value);
        rest = remainder;
    }

    if !rest.is_empty() {
        return None;
    }
    Some((x, y))
}

/// Parses the `G00`/`G01` prefix, ignoring anything after the coordinates.
fn route<'a>(line: &'a str, code: &str) -> Option<(Option<&'a str>, Option<&'a str>)> {
    let mut rest = line.strip_prefix(code)?;
    let mut x = None;
    let mut y = None;

    if let Some(after) = rest.strip_prefix('X')
        && let Some((value, remainder)) = take_coordinate(after)
    {
        x = Some(value);
        rest = remainder;
    }
    if let Some(after) = rest.strip_prefix('Y')
        && let Some((value, remainder)) = take_coordinate(after)
    {
        y = Some(value);
        rest = remainder;
    }

    let _ = rest;
    Some((x, y))
}

fn file_format(line: &str) -> Option<(u32, u32)> {
    let rest = &line[line.find("FILE_FORMAT=")? + "FILE_FORMAT=".len()..];
    let (integer, rest) = take_digits(rest)?;
    let rest = rest.strip_prefix(':')?;
    let (decimal, _) = take_digits(rest)?;
    Some((integer as u32, decimal as u32))
}

fn altium_tool(line: &str) -> Option<(u32, f64)> {
    let rest = line.strip_prefix('T')?;
    let (number, rest) = take_digits(rest)?;
    let rest = rest.strip_prefix('F')?;
    let (_, rest) = take_digits(rest)?;
    let rest = rest.strip_prefix('S')?;
    let (_, rest) = take_digits(rest)?;
    let rest = rest.strip_prefix('C')?;
    let (diameter, _) = take_number(rest)?;
    Some((number as u32, diameter))
}

fn kicad_tool(line: &str) -> Option<(u32, f64)> {
    let rest = line.strip_prefix('T')?;
    let (number, rest) = take_digits(rest)?;
    let rest = rest.strip_prefix('C')?;
    let (diameter, _) = take_number(rest)?;
    Some((number as u32, diameter))
}

// --- Altium ----------------------------------------------------------------

fn parse_altium(content: &str) -> Vec<Tool> {
    let mut unit = Unit::Metric;
    let mut format = (2u32, 4u32);
    let mut leading_zero = true;
    let mut plated = true;
    let mut tools: BTreeMap<u32, Tool> = BTreeMap::new();
    let mut in_header = true;

    for line in content.lines() {
        let line = line.trim();

        if line == "%" {
            in_header = false;
            continue;
        }
        if !in_header {
            continue;
        }

        let upper = line.to_ascii_uppercase();
        if upper.starts_with("INCH") {
            unit = Unit::Inch;
            if upper.contains("LZ") {
                leading_zero = true;
            } else if upper.contains("TZ") {
                leading_zero = false;
            }
        } else if upper.starts_with("METRIC") {
            unit = Unit::Metric;
            if upper.contains("LZ") {
                leading_zero = true;
            } else if upper.contains("TZ") {
                leading_zero = false;
            }
        }

        if let Some(values) = file_format(line) {
            format = values;
        }

        if line.contains("TYPE=NON_PLATED") {
            plated = false;
        } else if line.contains("TYPE=PLATED") {
            plated = true;
        }

        if let Some((number, diameter)) = altium_tool(line) {
            tools.insert(
                number,
                Tool {
                    diameter: if unit == Unit::Inch {
                        diameter * INCH
                    } else {
                        diameter
                    },
                    plated,
                    hits: Vec::new(),
                },
            );
        }
    }

    let mut current: Option<u32> = None;
    let mut routing = false;
    let mut last = (0.0f64, 0.0f64);
    let mut in_header = true;

    for line in content.lines() {
        let line = line.trim();

        if line == "%" {
            in_header = false;
            continue;
        }
        if in_header || line.is_empty() || line.starts_with(';') {
            continue;
        }

        if let Some(number) = tool_select(line) {
            if tools.contains_key(&number) {
                current = Some(number);
            }
            continue;
        }

        if let Some((x, y)) = route(line, "G00") {
            if let Some(raw) = x {
                last.0 = altium_coordinate(raw, format, leading_zero, unit);
            }
            if let Some(raw) = y {
                last.1 = altium_coordinate(raw, format, leading_zero, unit);
            }
            continue;
        }

        if line == "M15" {
            routing = true;
            continue;
        }

        if routing && let Some((x, y)) = route(line, "G01") {
            let start = last;
            if let Some(raw) = x {
                last.0 = altium_coordinate(raw, format, leading_zero, unit);
            }
            if let Some(raw) = y {
                last.1 = altium_coordinate(raw, format, leading_zero, unit);
            }
            if let Some(tool) = current.and_then(|number| tools.get_mut(&number)) {
                tool.hits.push(Hit::Slot {
                    x1: start.0,
                    y1: start.1,
                    x2: last.0,
                    y2: last.1,
                });
            }
            continue;
        }

        if line == "M16" {
            routing = false;
            continue;
        }

        if let Some((x, y)) = coordinate(line) {
            if x.is_none() && y.is_none() {
                continue;
            }
            if let Some(tool) = current.and_then(|number| tools.get_mut(&number)) {
                if let Some(raw) = x {
                    last.0 = altium_coordinate(raw, format, leading_zero, unit);
                }
                if let Some(raw) = y {
                    last.1 = altium_coordinate(raw, format, leading_zero, unit);
                }
                tool.hits.push(Hit::Hole {
                    x: last.0,
                    y: last.1,
                });
            }
        }
    }

    tools
        .into_values()
        .filter(|tool| !tool.hits.is_empty())
        .collect()
}

fn altium_coordinate(raw: &str, format: (u32, u32), leading_zero: bool, unit: Unit) -> f64 {
    if raw.contains('.') {
        let value: f64 = raw.parse().unwrap_or(0.0);
        return if unit == Unit::Inch {
            value * INCH
        } else {
            value
        };
    }

    let sign = if raw.starts_with('-') { -1.0 } else { 1.0 };
    let magnitude = raw.trim_start_matches(['+', '-']);

    let value = if leading_zero {
        let integer_places = format.0 as usize;
        if magnitude.len() <= integer_places {
            magnitude.parse::<f64>().unwrap_or(0.0)
        } else {
            let integer_part = &magnitude[..integer_places];
            let decimal_part = &magnitude[integer_places..];
            let integer: f64 = integer_part.parse().unwrap_or(0.0);
            let decimal = if decimal_part.is_empty() {
                0.0
            } else {
                decimal_part.parse::<f64>().unwrap_or(0.0) / 10_f64.powi(decimal_part.len() as i32)
            };
            integer + decimal
        }
    } else {
        magnitude
            .parse::<i64>()
            .map(|value| value as f64 / 10_f64.powi(format.1 as i32))
            .unwrap_or(0.0)
    };

    let value = sign * value;
    if unit == Unit::Inch {
        value * INCH
    } else {
        value
    }
}

// --- KiCad -----------------------------------------------------------------

fn parse_kicad(content: &str) -> Vec<Tool> {
    let plated = !(content.contains("NonPlated") || content.contains("NPTH"));
    let mut tools: BTreeMap<u32, Tool> = BTreeMap::new();
    let mut in_header = true;

    for line in content.lines() {
        let line = line.trim();
        if line == "%" {
            in_header = false;
            continue;
        }
        if !in_header {
            continue;
        }

        if let Some((number, diameter)) = kicad_tool(line) {
            tools.insert(
                number,
                Tool {
                    diameter,
                    plated,
                    hits: Vec::new(),
                },
            );
        }
    }

    let mut current: Option<u32> = None;
    let mut routing = false;
    let mut route_start: Option<(f64, f64)> = None;
    let mut last_y = 0.0f64;
    let mut in_header = true;

    for line in content.lines() {
        let line = line.trim();

        if line == "%" {
            in_header = false;
            continue;
        }
        if in_header || line.is_empty() || line.starts_with(';') {
            continue;
        }

        if let Some(number) = tool_select(line) {
            if tools.contains_key(&number) {
                current = Some(number);
            }
            continue;
        }

        if let Some((x, y)) = route(line, "G00") {
            let x: f64 = x.and_then(|raw| raw.parse().ok()).unwrap_or(0.0);
            let y: f64 = y.and_then(|raw| raw.parse().ok()).unwrap_or(0.0);
            route_start = Some((x, y));
            last_y = y;
            continue;
        }

        if line == "M15" {
            routing = true;
            continue;
        }

        if routing && let Some((x, y)) = route(line, "G01") {
            let end_x: f64 = x.and_then(|raw| raw.parse().ok()).unwrap_or(0.0);
            let end_y: f64 = y.and_then(|raw| raw.parse().ok()).unwrap_or(last_y);

            if let (Some(number), Some((start_x, start_y))) = (current, route_start)
                && let Some(tool) = tools.get_mut(&number)
            {
                tool.hits.push(Hit::Slot {
                    x1: start_x,
                    y1: start_y,
                    x2: end_x,
                    y2: end_y,
                });
            }
            last_y = end_y;
            continue;
        }

        if line == "M16" {
            routing = false;
            route_start = None;
            continue;
        }

        if line.starts_with('X') && line.contains('Y') {
            let rest = &line[1..];
            if let Some((x, rest)) = take_coordinate(rest)
                && let Some(rest) = rest.strip_prefix('Y')
                && let Some((y, _)) = take_coordinate(rest)
                && let (Ok(x), Ok(y)) = (x.parse::<f64>(), y.parse::<f64>())
                && let Some(tool) = current.and_then(|number| tools.get_mut(&number))
            {
                tool.hits.push(Hit::Hole { x, y });
            }
        }
    }

    tools
        .into_values()
        .filter(|tool| !tool.hits.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_altium_metric_drills() {
        let content = "M48\n;FILE_FORMAT=4:4\nMETRIC,LZ\n;TYPE=PLATED\nT01F00S00C0.3000\n%\nT01\nX0033086Y0002907\nM30\n";
        let tools = parse_altium(content);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].diameter, 0.3);
        assert_eq!(tools[0].hits.len(), 1);
        assert_eq!(
            tools[0].hits[0],
            Hit::Hole {
                x: 33.086,
                y: 2.907
            }
        );
    }

    #[test]
    fn parses_altium_non_plated_tools() {
        let content = "M48\n;FILE_FORMAT=4:4\nMETRIC,LZ\n;TYPE=NON_PLATED\nT04F00S00C0.9000\n%\nT04\nX0010000Y0010000\nM30\n";
        let tools = parse_altium(content);
        assert_eq!(tools.len(), 1);
        assert!(!tools[0].plated);
    }

    #[test]
    fn parses_kicad_slots() {
        let content = "# DRILL file {KiCad}\nM48\nMETRIC\nT1C0.500\n%\nG90\nG05\nT1\nG00X10.000Y10.000\nM15\nG01X12.000Y10.000\nM16\nM30\n";
        let tools = parse_kicad(content);
        assert_eq!(tools.len(), 1);
        assert!(matches!(tools[0].hits[0], Hit::Slot { .. }));
    }
}
