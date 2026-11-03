# jber — clean-room conversion specification

This document specifies the behaviour of a Gerber post-processor for JLCPCB
orders. It is the **only** input an implementer may use. Do not consult any
existing implementation (in particular, nothing under
`C:\Users\Admin\Desktop\nextjlc` or the engine files in
`C:\Users\Admin\Desktop\jber\src`); the CLI wrapper `src/main.rs` in this
crate is provided as-is and may be read only to learn the API it expects.

The implementation must be original Rust code written from this document. The
goal is byte-compatible *artwork and file layout*, not textual resemblance to
any other program.

---

## 1. Goal

Turn an archive exported by Altium Designer, KiCad or EasyEDA into a normalized
set that a PCB fab accepts:

* canonical `Gerber_*` layer names,
* one shared EasyEDA-style comment header per run,
* D-codes prefixed with `G54`,
* one added "tracking" aperture whose size derives from a hash,
* Altium/KiCad drill sources merged into one plated and one non-plated Excellon
  file,
* a short Chinese order guide,
* validation of the layer stack.

Header values (vendor, version, timestamp) and the tracking aperture size vary
between runs; everything else is deterministic and must match the reference
tool's output exactly.

## 2. Input

* The CLI accepts a `.zip` file or a directory.
* Every regular file is read as raw bytes.
* Archive entries are reduced to their final path component; directories are
  not recreated. Skip entries under `__MACOSX/`, names starting `._`,
  `.DS_Store` and `Thumbs.db`.
* Files are keyed by base name; on duplicates the last one read wins.
* The conversion processes names in ascending lexicographic byte order.

## 3. Source detection

For each file take the first ten lines of a lossy UTF-8 decode of its first
4096 bytes, lowercase it, and pick the first match:

1. contains `altium` → Altium
2. contains `kicad` → KiCad
3. contains `easyeda` → EasyEDA
4. otherwise none

The *primary* file decides how the whole archive is treated. Stable-sort the
names by this rank and take the first:

| rank | condition |
| --- | --- |
| 0 | name contains `Edge_Cuts` |
| 1 | name contains `F_Cu` |
| 2 | name contains `F_Mask` |
| 3 | lowercase name ends with `.gto` |
| 4 | … `.gtl` |
| 5 | … `.gbl` |
| 6 | … `.gm13` |
| 7 | … `.gm1` |
| 8 | … `.gko` |
| `usize::MAX` | anything else |

If the primary file has no detected source, abort with an error whose message
contains the file name and the detected label (`None`).

## 4. File naming

Altium and EasyEDA archives use Altium rules. Apply the first matching rule to
the lowercased full name:

| match | output |
| --- | --- |
| ends `.gm1` or `.gm13` | `Gerber_BoardOutlineLayer.GKO` |
| ends `.gm`, and the archive has no `.gm1`/`.gm13` | `Gerber_BoardOutlineLayer.GKO` |
| ends `.gm`, otherwise | `Gerber_DocumentLayer.GBR` |
| ends `.gtl` | `Gerber_TopLayer.GTL` |
| ends `.gto` | `Gerber_TopSilkscreenLayer.GTO` |
| ends `.gts` | `Gerber_TopSolderMaskLayer.GTS` |
| ends `.gtp` | `Gerber_TopPasteMaskLayer.GTP` |
| ends `.gbl` | `Gerber_BottomLayer.GBL` |
| ends `.gbo` | `Gerber_BottomSilkscreenLayer.GBO` |
| ends `.gbs` | `Gerber_BottomSolderMaskLayer.GBS` |
| ends `.gbp` | `Gerber_BottomPasteMaskLayer.GBP` |
| ends `.g1` … `.g6` | `Gerber_InnerLayer1.G1` … `Gerber_InnerLayer6.G6` |
| ends `.gp1` … `.gp6` | `Gerber_InnerLayer1.G1` … `Gerber_InnerLayer6.G6` |
| ends `.gd1` | `Drill_PTH_Through_GBR.GBR` |
| ends `.gg1` | `Drill_PTH_Through_Via_GBR.GBR` |
| ends `.drr` | `Drill_MAP_GBR.DRR` |
| ends `.ldp` | `Gerber_GBR_JOB.LDP` |
| ends `.txt` and contains `hole` | `Drill_Source.DRL` |
| ends `.tx1` … `.tx9` | `Drill_Source.DRL` |

KiCad rules, first match wins on the lowercased name:

| match | output |
| --- | --- |
| contains `edge_cuts` | `Gerber_BoardOutlineLayer.GKO` |
| contains `gm` | `Gerber_DocumentLayer.GBR` |
| contains `f_cu` | `Gerber_TopLayer.GTL` |
| contains `f_silkscreen` | `Gerber_TopSilkscreenLayer.GTO` |
| contains `f_mask` | `Gerber_TopSolderMaskLayer.GTS` |
| contains `f_paste` | `Gerber_TopPasteMaskLayer.GTP` |
| contains `b_cu` | `Gerber_BottomLayer.GBL` |
| contains `b_silkscreen` | `Gerber_BottomSilkscreenLayer.GBO` |
| contains `b_mask` | `Gerber_BottomSolderMaskLayer.GBS` |
| contains `b_paste` | `Gerber_BottomPasteMaskLayer.GBP` |
| contains `in1_cu` … `in6_cu` | `Gerber_InnerLayer1.G1` … `Gerber_InnerLayer6.G6` |
| ends `.drl` | `Drill_Source.DRL` |
| ends `-drl_map`, `_drl_map`, `-drl_map.gbr`, `_drl_map.gbr` | `Drill_MAP_GBR.GBR` |
| ends `.gbrjob` | `Gerber_GBR_JOB.GBRJOB` |

Unmatched files keep their original name. A file is an *inner layer* when its
output name starts with `Gerber_InnerLayer`.

## 5. Gerber processing

Files whose source was detected get, in this order:

1. a comment header prepended,
2. `G54` normalization of D-codes,
3. tracking-aperture injection.

Files with no detected source are copied byte-for-byte and only renamed.

### 5.1 Comment header

Generate once per run and prepend to every detected Gerber file:

```
G04 {vendor} {version}, {timestamp}*
G04 Gerber Generator version 0.3*
```

* vendor: `EasyEDA Pro` or `EasyEDA`, chosen at random.
* version: `v{major}.{minor}.{patch}.{build}` with major 2–3, minor 1–5,
  patch 1–42, build 0–2, each uniform.
* timestamp: local time formatted `%Y-%m-%d %H:%M:%S`.

Randomness must differ between runs; no fixed seed is exposed.

### 5.2 D-code normalization

Process line by line, preserving line endings as `\n` (join with `\n`).

A line is *skipped* when:

* Altium/EasyEDA: it contains `G54D`; or it starts with `G01`, `G02`, `G36`
  or `G37` and its trimmed form is `D01*`, `D02*`, `D03*`, or contains `X` or
  `Y`.
* KiCad: it contains `%ADD` or `G54D`.

Otherwise, if the line starts with `D` followed by 2–4 digits and `*`, insert
`G54` before that token. Do not touch anything else on the line.

### 5.3 Negative-plane notice

If an *inner layer* contains `filepolarity,negative` (case-insensitive), leave
the artwork untouched and collect its output name. After processing, emit one
warning listing them, e.g.

```
negative plane layers kept as-is: Gerber_InnerLayer1.G1, Gerber_InnerLayer2.G2
```

No geometry may be rewritten.

### 5.4 Tracking aperture

Used to make each file's hash differ; must be implemented exactly.

1. Normalize `\r\n` to `\n`; strip one leading BOM. If the text exceeds
   30,000,000 bytes, skip.
2. Scan the first 200 lines (trimmed) for `%ADD` + 2–4 digits + a non-digit.
   Collect `(id, line)` in order. If fewer than 5 definitions, skip.
3. Group by id. Pick the id at index 5 of the sorted distinct ids (or the
   largest if there are fewer). Call it `id`.
4. `template` = lexicographically smallest definition line of `id`.
5. Shift every line starting with `%ADD` or `G54D`: read up to four digits after
   the prefix; if the number is ≥ `id`, add 1 to it; rebuild the line.
6. Compute MD5 over the shifted text's bytes (UTF-8). Let `n = last_byte % 100`.
   `size = format!("0.42{:02}", n)`.
7. Build the stamped line from `template`: replace the first `,` followed by
   digits/dots with `,{size}`; if there is no such comma, use
   `%ADD{id}C,{size}*%`.
8. Insert it after the first line starting `%ADD{id-1}`. If none, insert after
   the `%MO…` line, immediately before the first following line that starts
   with `%LP` or `G`. If neither, append at the end.
9. Join with `\n`.

## 6. Drills

### 6.1 Recognising drill files

A name is a drill source when (lowercased) it ends with `.drl`, or ends with
`.txt` and contains `hole` or `drill`, or ends with `.tx1`…`.tx6`.

`.tx1`–`.tx6` are blind/buried and must be skipped with a warning such as
`skipping {name}: blind/buried vias are not supported`.

### 6.2 Altium parser

State: unit (metric default), format (2 integer, 4 decimal by default),
leading-zero mode (true), current plating (plated).

Header pass, until the first line `%`:

* upper begins `INCH` → inch; `METRIC` → metric; `LZ` sets leading-zero true,
  `TZ` false.
* `FILE_FORMAT=a:b` sets the format.
* `TYPE=NON_PLATED` → non-plated; `TYPE=PLATED` → plated.
* tool definition `T<number>F<digits>S<digits>C<number>` adds a tool with that
  diameter (multiply by 25.4 when inch).

Command pass, skipping until `%` again, then ignoring blank lines and lines
starting `;`:

* `T<number>` alone selects a known tool.
* `G00` with optional `X…`/`Y…` updates the current position and continues.
* `M15` starts routing, `M16` stops.
* while routing, `G01` with optional `X…`/`Y…` emits a slot from the previous
  position to the new one.
* any other line that consists only of optional `X…`/`Y…` emits a hole.
* Coordinates: if they contain `.`, parse as millimetres (or inches);
  otherwise with leading zeros: split after `a` digits, integer part plus
  fractional part scaled by 10^len; with trailing zeros: integer divided by
  10^b. Apply sign, convert inches.

Keep only tools that produced hits.

### 6.3 KiCad parser

Plated unless the content contains `NonPlated` or `NPTH`.

Header until `%`: `T<number>C<number>` defines a tool (millimetres).

Commands: tool select as above; `G00` sets route start and last Y; `M15` starts
routing; `G01` while routing emits a slot; `M16` resets. Lines starting with
`X` and containing `Y` emit a hole parsed as decimal millimetres.

### 6.4 Merge

Merge all plated tools and all non-plated tools separately. Tools with the same
diameter (`round(diameter * 100000)` as key) merge their hits. Order by that
key ascending.

### 6.5 Emit

For each non-empty group:

```
;TYPE={PLATED|NON_PLATED}
;Layer: {PTH_Through|NPTH_Through}
;{vendor} {version}, {timestamp}
;Gerber Generator version 0.3
M48
METRIC,LZ,0000.00000
;Hole size {n} = {diameter:.5} METRIC
T{n:02}C{diameter:.5}
...
%
G05
G90
T{n:02}
X{x:.5}Y{y:.5}
X{x1:.5}Y{y1:.5}G85X{x2:.5}Y{y2:.5}
...
M30
```

One blank-free block per tool; `n` starts at 1 in key order. The header values
may be regenerated per file.

Output names: `Drill_PTH_Through.DRL` and `Drill_NPTH_Through.DRL`.

## 7. Validation

Required prefixes: `Gerber_BoardOutlineLayer`, `Gerber_TopLayer`,
`Gerber_TopSolderMaskLayer`. Any missing one is a fatal error.

Copper layer count = top + bottom + number of `Gerber_InnerLayer*` files.

Errors (fatal):

* top copper without top solder mask,
* bottom copper without bottom solder mask,
* top plus inner copper without bottom copper.

Warnings (non-fatal): missing top silkscreen, top paste, bottom silkscreen,
bottom paste. Wording is free.

## 8. Output

* Name: `<input stem>-<AD|Ki|ED>-L<layers>.zip`, where AD = Altium, Ki = KiCad,
  ED = EasyEDA, next to the input; `-o` overrides.
* `-o` ending in `.zip` writes an archive; any other path is a directory.
* Existing output is refused unless `--force`.
* `--clean` keeps only manufacturing plots (`Gerber_*` with these suffixes:
  GTL, GTO, GTS, GTP, GBL, GBO, GBS, GBP, G1–G6, GKO; and the two merged drill
  files) plus the guide.
* Always add `PCB下单必读.txt` unless already present, containing exactly:

```
PCB 下单说明

上传本压缩包即可下单。
参考：
https://prodocs.lceda.cn/cn/pcb/order-order-pcb/index.html
```

## 9. API contract

`src/main.rs` is fixed and expects:

```rust
// crate root
pub mod eda;
pub mod excellon;
pub mod pipeline;

// eda
pub enum Eda { Altium, KiCad, EasyEDA }
impl Eda {
    pub fn label(self) -> &'static str; // "Altium" | "KiCad" | "EasyEDA"
    pub fn tag(self) -> &'static str;   // "AD" | "Ki" | "ED"
}

// excellon
pub fn is_drill_file(name: &str) -> bool;

// pipeline
pub struct Settings { pub clean: bool }      // Default
pub struct Output { pub name: String, pub data: Vec<u8>, pub plot: bool }
pub struct Conversion {
    pub outputs: Vec<Output>,
    pub mapping: std::collections::BTreeMap<String, String>,
    pub warnings: Vec<String>,
    pub eda: Eda,
    pub layers: u32,
    pub renamed: usize,
    pub kept: usize,
    pub drill_sources: usize,
}
pub fn convert(
    files: std::collections::BTreeMap<String, Vec<u8>>,
    settings: &Settings,
) -> Result<Conversion, String>;
```

Other modules may exist privately. `pipeline::convert` must return the outputs
in the order implied by the processing rules (renamed plots in name order,
then PTH drill, then NPTH drill, then guide) so the archive listing is stable.

## 10. Acceptance

On `fixtures/input.zip` (an Altium 4-layer board):

* exit 0, output `…-AD-L4.zip` with 23 entries in `--clean=false` mode;
* mapping contains `main.GM → Gerber_BoardOutlineLayer.GKO`,
  `main.GP1 → Gerber_InnerLayer1.G1`, `main.GP2 → Gerber_InnerLayer2.G2`;
* merged PTH drill has 151 `X…` hole lines and 4 lines containing `G85`;
  NPTH has 2 holes;
* every Gerber file starts with the two-line header and every drill file with
  the four-line header;
* one warning mentions the two negative inner planes.

Run `cargo build --release` and `cargo test`; add unit tests for naming,
D-code rules, the tracking aperture and both drill dialects.
