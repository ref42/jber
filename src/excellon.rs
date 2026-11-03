//! Excellon drill parsing, merging and emission.
//!
//! Two dialects are supported: the Altium export (`M48` header with
//! `T..F..S..C..` tool definitions and zero-suppressed coordinates) and the
//! KiCad export (decimal millimetre coordinates).

use std::collections::BTreeMap;

/// Whether a drill tool belongs to the plated or non-plated group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Plating {
    Plated,
    NonPlated,
}

/// One drill hit. A round hole has identical start and end points; a slot is a
/// routed segment between them.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Hit {
    pub from: (f64, f64),
    pub to: (f64, f64),
}

impl Hit {
    fn hole(x: f64, y: f64) -> Self {
        Hit {
            from: (x, y),
            to: (x, y),
        }
    }

    fn slot(from: (f64, f64), to: (f64, f64)) -> Self {
        Hit { from, to }
    }

    fn is_hole(self) -> bool {
        self.from == self.to
    }
}

/// A drill tool together with every hit made with it.
#[derive(Clone, Debug)]
pub(crate) struct Tool {
    pub diameter: f64,
    pub plating: Plating,
    pub hits: Vec<Hit>,
}

/// A name is a drill source when it is a `.drl`, a `.txt` whose name mentions
/// `hole`/`drill`, or a blind/buried `.tx1`–`.tx6` plot.
pub fn is_drill_file(name: &str) -> bool {
    let lower = name.to_lowercase();
    if lower.ends_with(".drl") {
        return true;
    }
    if lower.ends_with(".txt") && (lower.contains("hole") || lower.contains("drill")) {
        return true;
    }
    (1..=6).any(|index| lower.ends_with(&format!(".tx{index}")))
}

/// `.tx1`–`.tx6` plots are blind/buried via layers, which JLCPCB cannot use.
pub(crate) fn is_blind_buried(name: &str) -> bool {
    let lower = name.to_lowercase();
    (1..=6).any(|index| lower.ends_with(&format!(".tx{index}")))
}

// --- Altium ----------------------------------------------------------------

pub(crate) fn parse_altium(text: &str) -> Vec<Tool> {
    let mut lines = text.lines();

    let mut unit = 1.0_f64; // millimetres
    let mut integer_digits = 2_usize;
    let mut decimal_digits = 4_usize;
    let mut leading_zeros = true;
    let mut plating = Plating::Plated;
    let mut tools: BTreeMap<u32, Tool> = BTreeMap::new();

    // Header: everything before the first bare `%` line.
    for raw in lines.by_ref() {
        let line = raw.trim();
        if line == "%" {
            break;
        }

        let upper = line.to_ascii_uppercase();
        if upper.starts_with("INCH") {
            unit = 25.4;
        } else if upper.starts_with("METRIC") {
            unit = 1.0;
        }
        if upper.contains("LZ") {
            leading_zeros = true;
        }
        if upper.contains("TZ") {
            leading_zeros = false;
        }
        if let Some(position) = upper.find("FILE_FORMAT=") {
            let spec = &upper[position + "FILE_FORMAT=".len()..];
            if let Some((left, right)) = spec.split_once(':') {
                let left = left.trim();
                let right = right.trim().trim_end_matches('*').trim();
                if let (Ok(a), Ok(b)) = (left.parse::<usize>(), right.parse::<usize>()) {
                    integer_digits = a;
                    decimal_digits = b;
                }
            }
        }
        if upper.contains("TYPE=NON_PLATED") {
            plating = Plating::NonPlated;
        } else if upper.contains("TYPE=PLATED") {
            plating = Plating::Plated;
        }
        if let Some((number, diameter)) = altium_tool_definition(line) {
            tools.insert(
                number,
                Tool {
                    diameter: diameter * unit,
                    plating,
                    hits: Vec::new(),
                },
            );
        }
    }

    // Command section: a small Excellon interpreter.
    let mut current: Option<u32> = None;
    let mut position = (0.0_f64, 0.0_f64);
    let mut routing = false;

    for raw in lines {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }

        if let Some(number) = tool_select(line) {
            if tools.contains_key(&number) {
                current = Some(number);
            }
            continue;
        }

        if let Some(rest) = line.strip_prefix("G00") {
            if let Some(tokens) = xy_tokens(rest)
                && let Some(next) = altium_position(
                    &tokens,
                    position,
                    integer_digits,
                    decimal_digits,
                    leading_zeros,
                    unit,
                )
            {
                position = next;
            }
            continue;
        }
        if line.starts_with("M15") {
            routing = true;
            continue;
        }
        if line.starts_with("M16") {
            routing = false;
            continue;
        }

        if let Some(rest) = line.strip_prefix("G01") {
            if routing
                && let Some(next) = xy_tokens(rest).and_then(|tokens| {
                    altium_position(
                        &tokens,
                        position,
                        integer_digits,
                        decimal_digits,
                        leading_zeros,
                        unit,
                    )
                })
            {
                if let Some(tool) = current.and_then(|number| tools.get_mut(&number)) {
                    tool.hits.push(Hit::slot(position, next));
                }
                position = next;
            }
            continue;
        }

        // Any other line that consists only of optional X/Y tokens is a hole.
        if let Some(next) = xy_tokens(line).and_then(|tokens| {
            altium_position(
                &tokens,
                position,
                integer_digits,
                decimal_digits,
                leading_zeros,
                unit,
            )
        }) {
            if let Some(tool) = current.and_then(|number| tools.get_mut(&number)) {
                tool.hits.push(Hit::hole(next.0, next.1));
            }
            position = next;
        }
    }

    tools.into_values().collect()
}

/// Aperture definition of the form `T<number>F<digits>S<digits>C<number>`.
fn altium_tool_definition(line: &str) -> Option<(u32, f64)> {
    let rest = line.strip_prefix('T')?;
    let number_end = rest.find(|c: char| !c.is_ascii_digit())?;
    if number_end == 0 {
        return None;
    }
    let number: u32 = rest[..number_end].parse().ok()?;

    let rest = rest[number_end..].strip_prefix('F')?;
    let rest = rest.trim_start_matches(|c: char| c.is_ascii_digit());
    let rest = rest.strip_prefix('S')?;
    let rest = rest.trim_start_matches(|c: char| c.is_ascii_digit());
    let rest = rest.strip_prefix('C')?;
    let diameter: f64 = rest.trim().trim_end_matches('*').trim().parse().ok()?;
    Some((number, diameter))
}

fn tool_select(line: &str) -> Option<u32> {
    let rest = line.strip_prefix('T')?;
    if rest.is_empty() || !rest.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    rest.parse().ok()
}

fn altium_position(
    tokens: &[(char, &str)],
    current: (f64, f64),
    integer_digits: usize,
    decimal_digits: usize,
    leading_zeros: bool,
    unit: f64,
) -> Option<(f64, f64)> {
    let mut position = current;
    for &(axis, token) in tokens {
        let value = altium_coordinate(token, integer_digits, decimal_digits, leading_zeros, unit)?;
        if axis == 'X' {
            position.0 = value;
        } else {
            position.1 = value;
        }
    }
    Some(position)
}

fn altium_coordinate(
    token: &str,
    integer_digits: usize,
    decimal_digits: usize,
    leading_zeros: bool,
    unit: f64,
) -> Option<f64> {
    let (sign, body) = if let Some(rest) = token.strip_prefix('-') {
        (-1.0, rest)
    } else if let Some(rest) = token.strip_prefix('+') {
        (1.0, rest)
    } else {
        (1.0, token)
    };

    if body.contains('.') {
        let value: f64 = body.parse().ok()?;
        return Some(sign * value * unit);
    }
    if body.is_empty() || !body.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }

    let value = if leading_zeros {
        if body.len() > integer_digits {
            let (whole_text, fraction_text) = body.split_at(integer_digits);
            let whole = whole_text.parse::<f64>().ok()?;
            let fraction =
                fraction_text.parse::<f64>().ok()? / 10_f64.powi(fraction_text.len() as i32);
            whole + fraction
        } else {
            body.parse::<f64>().ok()?
        }
    } else {
        let raw: f64 = body.parse().ok()?;
        raw / 10_f64.powi(decimal_digits as i32)
    };

    Some(sign * value * unit)
}

// --- KiCad -----------------------------------------------------------------

pub(crate) fn parse_kicad(text: &str) -> Vec<Tool> {
    let plating = if text.contains("NonPlated") || text.contains("NPTH") {
        Plating::NonPlated
    } else {
        Plating::Plated
    };

    let mut lines = text.lines();
    let mut tools: BTreeMap<u32, Tool> = BTreeMap::new();

    for raw in lines.by_ref() {
        let line = raw.trim();
        if line == "%" {
            break;
        }
        if let Some((number, diameter)) = kicad_tool_definition(line) {
            tools.insert(
                number,
                Tool {
                    diameter,
                    plating,
                    hits: Vec::new(),
                },
            );
        }
    }

    let mut current: Option<u32> = None;
    let mut position = (0.0_f64, 0.0_f64);
    let mut routing = false;

    for raw in lines {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }

        if let Some(number) = tool_select(line) {
            if tools.contains_key(&number) {
                current = Some(number);
            }
            continue;
        }

        if let Some(rest) = line.strip_prefix("G00") {
            if let Some(next) =
                xy_tokens(rest).and_then(|tokens| decimal_position(&tokens, position))
            {
                position = next;
            }
            continue;
        }
        if line.starts_with("M15") {
            routing = true;
            continue;
        }
        if line.starts_with("M16") {
            routing = false;
            continue;
        }

        if let Some(rest) = line.strip_prefix("G01") {
            if routing
                && let Some(next) =
                    xy_tokens(rest).and_then(|tokens| decimal_position(&tokens, position))
            {
                if let Some(tool) = current.and_then(|number| tools.get_mut(&number)) {
                    tool.hits.push(Hit::slot(position, next));
                }
                position = next;
            }
            continue;
        }

        if line.starts_with('X')
            && line.contains('Y')
            && let Some(next) =
                xy_tokens(line).and_then(|tokens| decimal_position(&tokens, position))
        {
            if let Some(tool) = current.and_then(|number| tools.get_mut(&number)) {
                tool.hits.push(Hit::hole(next.0, next.1));
            }
            position = next;
        }
    }

    tools.into_values().collect()
}

fn kicad_tool_definition(line: &str) -> Option<(u32, f64)> {
    let rest = line.strip_prefix('T')?;
    let split = rest.find('C')?;
    let number: u32 = rest[..split].parse().ok()?;
    let diameter: f64 = rest[split + 1..]
        .trim()
        .trim_end_matches('*')
        .trim()
        .parse()
        .ok()?;
    Some((number, diameter))
}

fn decimal_position(tokens: &[(char, &str)], current: (f64, f64)) -> Option<(f64, f64)> {
    let mut position = current;
    for &(axis, token) in tokens {
        let value: f64 = token.parse().ok()?;
        if axis == 'X' {
            position.0 = value;
        } else {
            position.1 = value;
        }
    }
    Some(position)
}

// --- Shared parsing helpers ------------------------------------------------

/// Split a command remainder into `X...`/`Y...` tokens. Returns `None` when
/// anything other than coordinate tokens is present.
fn xy_tokens(text: &str) -> Option<Vec<(char, &str)>> {
    let mut rest = text.trim();
    if rest.is_empty() {
        return None;
    }

    let mut tokens = Vec::new();
    while !rest.is_empty() {
        let axis = rest.chars().next()?;
        if axis != 'X' && axis != 'Y' {
            return None;
        }
        let tail = &rest[1..];
        let end = tail.find(['X', 'Y']).unwrap_or(tail.len());
        let token = tail[..end].trim().trim_end_matches('*').trim();
        if token.is_empty() {
            return None;
        }
        tokens.push((axis, token));
        rest = &tail[end..];
    }
    Some(tokens)
}

// --- Merge and emit --------------------------------------------------------

/// Merge every parsed tool into one plated and one non-plated list. Tools are
/// keyed by `round(diameter * 100000)` and sorted by that key.
pub(crate) fn merge(tools: Vec<Tool>) -> (Vec<Tool>, Vec<Tool>) {
    let mut plated: BTreeMap<i64, Tool> = BTreeMap::new();
    let mut non_plated: BTreeMap<i64, Tool> = BTreeMap::new();

    for tool in tools {
        if tool.hits.is_empty() {
            continue;
        }
        let key = (tool.diameter * 100_000.0).round() as i64;
        let group = match tool.plating {
            Plating::Plated => &mut plated,
            Plating::NonPlated => &mut non_plated,
        };
        match group.get_mut(&key) {
            Some(existing) => existing.hits.extend(tool.hits),
            None => {
                group.insert(
                    key,
                    Tool {
                        diameter: key as f64 / 100_000.0,
                        plating: tool.plating,
                        hits: tool.hits,
                    },
                );
            }
        }
    }

    (
        plated.into_values().collect(),
        non_plated.into_values().collect(),
    )
}

/// Render one merged drill file, using the shared run stamp.
pub(crate) fn render(tools: &[Tool], plating: Plating, stamp: &str) -> String {
    let (type_name, layer) = match plating {
        Plating::Plated => ("PLATED", "PTH_Through"),
        Plating::NonPlated => ("NON_PLATED", "NPTH_Through"),
    };

    let mut out = String::new();
    out.push_str(&format!(
        ";TYPE={type_name}\n;Layer: {layer}\n;{stamp}\n;Gerber Generator version 0.3\nM48\nMETRIC,LZ,0000.00000\n"
    ));

    for (index, tool) in tools.iter().enumerate() {
        let number = index + 1;
        out.push_str(&format!(
            ";Hole size {number} = {:.5} METRIC\nT{number:02}C{:.5}\n",
            tool.diameter, tool.diameter
        ));
    }

    out.push_str("%\nG05\nG90\n");

    for (index, tool) in tools.iter().enumerate() {
        out.push_str(&format!("T{:02}\n", index + 1));
        for hit in &tool.hits {
            if hit.is_hole() {
                out.push_str(&format!("X{}Y{}\n", coord(hit.to.0), coord(hit.to.1)));
            } else {
                out.push_str(&format!(
                    "X{}Y{}G85X{}Y{}\n",
                    coord(hit.from.0),
                    coord(hit.from.1),
                    coord(hit.to.0),
                    coord(hit.to.1)
                ));
            }
        }
    }

    out.push_str("M30\n");
    out
}

fn coord(value: f64) -> String {
    format!("{value:.5}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_drill_names() {
        assert!(is_drill_file("board.drl"));
        assert!(is_drill_file("main-RoundHoles.TXT"));
        assert!(is_drill_file("main-SlotHoles.TXT"));
        assert!(is_drill_file("main.TX1"));
        assert!(!is_drill_file("main.TX7"));
        assert!(!is_drill_file("Status Report.Txt"));
        assert!(!is_drill_file("main.GTL"));
    }

    #[test]
    fn altium_round_and_slot_hits() {
        let text = "\
M48
;FILE_FORMAT=4:4
METRIC,LZ
;TYPE=PLATED
T01F00S00C0.3000
;TYPE=NON_PLATED
T04F00S00C0.9000
%
T01
X0033086Y0002907
X00321535Y00102652
T04
X00369992Y00071361
Y00101361
M30
";
        let tools = parse_altium(text);
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].plating, Plating::Plated);
        assert_eq!(tools[0].hits.len(), 2);
        assert_eq!(tools[1].plating, Plating::NonPlated);
        assert_eq!(tools[1].hits.len(), 2);

        let (pth, npth) = merge(tools);
        assert_eq!(pth.len(), 1);
        assert_eq!(npth.len(), 1);
        assert!((pth[0].diameter - 0.3).abs() < 1e-9);
        assert!((npth[0].diameter - 0.9).abs() < 1e-9);
    }

    #[test]
    fn altium_slot_route() {
        let text = "\
M48
;FILE_FORMAT=4:4
METRIC,LZ
;TYPE=PLATED
T02F00S00C0.5000
%
G90
G05
T02
G00X00340041Y001438
M15
G01X00335041
M16
M30
";
        let tools = parse_altium(text);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].hits.len(), 1);
        let hit = tools[0].hits[0];
        assert!(!hit.is_hole());
        assert!((hit.from.0 - 34.0041).abs() < 1e-9);
        assert!((hit.from.1 - 14.38).abs() < 1e-9);
        assert!((hit.to.0 - 33.5041).abs() < 1e-9);
        assert!((hit.to.1 - 14.38).abs() < 1e-9);
    }

    #[test]
    fn altium_trailing_zeros_and_inches() {
        let text = "\
M48
INCH,TZ
;FILE_FORMAT=2:4
T01F00S00C0.0100
%
T01
X100000Y020000
M30
";
        let tools = parse_altium(text);
        assert_eq!(tools.len(), 1);
        let hit = tools[0].hits[0];
        // 10.0000 inch, 2.0000 inch, converted to millimetres.
        assert!((hit.to.0 - 254.0).abs() < 1e-9);
        assert!((hit.to.1 - 50.8).abs() < 1e-9);
        assert!((tools[0].diameter - 0.254).abs() < 1e-9);
    }

    #[test]
    fn kicad_decimal_holes() {
        let text = "\
M48
;DRILL file {KiCad 8.0}
FMAT,2
METRIC
T1C0.400
T2C0.800
%
G90
G05
T1
X10.500Y20.250
X11.000Y20.250
T2
X12.000Y20.250
M30
";
        let tools = parse_kicad(text);
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].hits.len(), 2);
        assert_eq!(tools[1].hits.len(), 1);
        assert!((tools[0].hits[1].to.0 - 11.0).abs() < 1e-9);
        assert!((tools[0].hits[1].to.1 - 20.25).abs() < 1e-9);
    }

    #[test]
    fn kicad_non_plated_and_slot() {
        let text = "\
M48
;DRILL file {KiCad 8.0} NonPlated
METRIC
T1C1.000
%
G90
G05
T1
G00X1.000Y2.000
M15
G01X4.000
M16
X5.000Y6.000
M30
";
        let tools = parse_kicad(text);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].plating, Plating::NonPlated);
        assert_eq!(tools[0].hits.len(), 2);
        assert!(!tools[0].hits[0].is_hole());
        assert!((tools[0].hits[0].to.0 - 4.0).abs() < 1e-9);
        assert!((tools[0].hits[0].to.1 - 2.0).abs() < 1e-9);
        assert!(tools[0].hits[1].is_hole());
    }

    #[test]
    fn merging_groups_equal_diameters() {
        let a = Tool {
            diameter: 0.3,
            plating: Plating::Plated,
            hits: vec![Hit::hole(1.0, 2.0)],
        };
        let b = Tool {
            diameter: 2.2,
            plating: Plating::Plated,
            hits: vec![Hit::hole(3.0, 4.0)],
        };
        let c = Tool {
            diameter: 0.3000001,
            plating: Plating::Plated,
            hits: vec![Hit::hole(5.0, 6.0)],
        };
        let (pth, npth) = merge(vec![b, c, a]);
        assert!(npth.is_empty());
        assert_eq!(pth.len(), 2);
        assert!((pth[0].diameter - 0.3).abs() < 1e-9);
        assert_eq!(pth[0].hits.len(), 2);
        assert_eq!(pth[1].hits.len(), 1);
    }

    #[test]
    fn renders_holes_and_slots() {
        let tools = vec![Tool {
            diameter: 0.5,
            plating: Plating::Plated,
            hits: vec![Hit::hole(1.0, 2.0), Hit::slot((1.0, 2.0), (1.5, 2.0))],
        }];
        let out = render(
            &tools,
            Plating::Plated,
            "EasyEDA v2.1.1.0, 2024-01-01 00:00:00",
        );
        assert!(out.starts_with(";TYPE=PLATED\n;Layer: PTH_Through\n"));
        assert!(out.contains("M48\nMETRIC,LZ,0000.00000\n"));
        assert!(out.contains(";Hole size 1 = 0.50000 METRIC\nT01C0.50000\n"));
        assert!(out.contains("X1.00000Y2.00000\n"));
        assert!(out.contains("X1.00000Y2.00000G85X1.50000Y2.00000\n"));
        assert!(out.ends_with("M30\n"));
    }
}
