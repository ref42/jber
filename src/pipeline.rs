/* src/pipeline.rs */

//! Conversion pipeline: name mapping, plane polarity, header stamping, D-code
//! normalization, drill merging and stack validation.

use std::collections::BTreeMap;

use crate::banner::Stamp;
use crate::eda::{self, Eda};
use crate::{banner, excellon, gerber, naming, stack, watermark};

pub const GUIDE_NAME: &str = "PCB下单必读.txt";

#[derive(Default)]
pub struct Settings {
    pub clean: bool,
}

pub struct Output {
    pub name: String,
    pub data: Vec<u8>,
    /// True for layers/drill files that belong in a manufacturing upload.
    pub plot: bool,
}

pub struct Conversion {
    pub outputs: Vec<Output>,
    pub mapping: BTreeMap<String, String>,
    pub warnings: Vec<String>,
    pub eda: Eda,
    pub layers: u32,
    pub renamed: usize,
    pub kept: usize,
    pub drill_sources: usize,
}

pub fn convert(
    files: BTreeMap<String, Vec<u8>>,
    settings: &Settings,
) -> Result<Conversion, String> {
    let names: Vec<String> = files.keys().cloned().collect();

    let mut software: BTreeMap<&str, Option<Eda>> = BTreeMap::new();
    for name in &names {
        let bytes = &files[name];
        let head = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]);
        let head: String = head.split('\n').take(10).collect::<Vec<_>>().join("\n");
        software.insert(name, eda::detect(&head));
    }

    let ordered = eda::order_for_primary(&names);
    let primary = ordered
        .first()
        .ok_or_else(|| "empty file list".to_string())?;
    let primary_eda = software[primary.as_str()].ok_or_else(|| {
        format!(
            "unsupported source: \"{primary}\" was identified as \"{}\" (Altium, KiCad and EasyEDA only)",
            software[primary.as_str()].map(Eda::label).unwrap_or("None")
        )
    })?;

    let mapping = naming::plan(&names, primary_eda);

    let mut stamp = Stamp::new();
    let mut outputs: Vec<Output> = Vec::new();
    let mut drill_sources: Vec<(String, String)> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut negative_planes: Vec<String> = Vec::new();

    for name in &names {
        let renamed = mapping.get(name).cloned().unwrap_or_else(|| name.clone());
        let raw = &files[name];

        if excellon::is_drill_file(name) {
            drill_sources.push((name.clone(), decode(raw)));
            continue;
        }

        let Some(source) = software[name.as_str()] else {
            // Not a plot: reports, drawings and unknown attachments keep their
            // bytes and their name.
            outputs.push(Output {
                name: renamed.clone(),
                data: raw.clone(),
                plot: is_plot(&renamed),
            });
            continue;
        };

        let mut content = decode(raw);

        // Altium marks solder-mask layers as negative too, but that is the
        // normal way to plot mask openings; only inner planes are genuinely
        // inverted images, so just note them and leave the artwork untouched.
        if renamed.starts_with("Gerber_InnerLayer") && gerber::is_negative(&content) {
            negative_planes.push(renamed.clone());
        }

        content = format!("{}{content}", stamp.gerber_header());
        content = gerber::normalize_dcodes(&content, flavor(source));
        content = watermark::embed(&content, false);

        outputs.push(Output {
            name: renamed.clone(),
            data: content.into_bytes(),
            plot: is_plot(&renamed),
        });
    }

    if !negative_planes.is_empty() {
        warnings.push(format!(
            "negative plane layer{} kept as-is: {} (TF.FilePolarity,Negative; check that your fab renders it correctly)",
            if negative_planes.len() == 1 { "" } else { "s" },
            negative_planes.join(", ")
        ));
    }

    let merged = excellon::merge(&drill_sources, &mut stamp);
    warnings.extend(merged.warnings);
    if let Some(plated) = merged.plated {
        outputs.push(Output {
            name: "Drill_PTH_Through.DRL".to_string(),
            data: plated.into_bytes(),
            plot: true,
        });
    }
    if let Some(non_plated) = merged.non_plated {
        outputs.push(Output {
            name: "Drill_NPTH_Through.DRL".to_string(),
            data: non_plated.into_bytes(),
            plot: true,
        });
    }

    let final_names: Vec<String> = outputs.iter().map(|output| output.name.clone()).collect();
    let stack = stack::check(&final_names)
        .map_err(|errors| format!("gerber validation failed:\n  - {}", errors.join("\n  - ")))?;
    warnings.extend(stack.warnings);

    if !outputs.iter().any(|output| output.name == GUIDE_NAME) {
        outputs.push(Output {
            name: GUIDE_NAME.to_string(),
            data: banner::order_guide().as_bytes().to_vec(),
            plot: false,
        });
    }

    if settings.clean {
        outputs.retain(|output| output.plot || output.name == GUIDE_NAME);
    }

    let (renamed, kept) = names
        .iter()
        .filter(|name| !excellon::is_drill_file(name))
        .fold((0, 0), |(renamed, kept), name| {
            if mapping.get(name.as_str()).map(String::as_str) == Some(name.as_str()) {
                (renamed, kept + 1)
            } else {
                (renamed + 1, kept)
            }
        });

    Ok(Conversion {
        outputs,
        mapping,
        warnings,
        eda: primary_eda,
        layers: stack.layers,
        renamed,
        kept,
        drill_sources: drill_sources.len(),
    })
}

fn decode(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn flavor(source: Eda) -> gerber::Flavor {
    match source {
        Eda::KiCad => gerber::Flavor::KiCad,
        Eda::Altium | Eda::EasyEDA => gerber::Flavor::Altium,
    }
}

/// Manufacturing layers and drills, as opposed to reports and drawings.
fn is_plot(name: &str) -> bool {
    if name == "Drill_PTH_Through.DRL" || name == "Drill_NPTH_Through.DRL" {
        return true;
    }

    const SUFFIXES: [&str; 15] = [
        ".GTL", ".GTO", ".GTS", ".GTP", ".GBL", ".GBO", ".GBS", ".GBP", ".G1", ".G2", ".G3", ".G4",
        ".G5", ".G6", ".GKO",
    ];

    let upper = name.to_ascii_uppercase();
    upper.starts_with("GERBER_") && SUFFIXES.iter().any(|suffix| upper.ends_with(suffix))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> Vec<u8> {
        value.as_bytes().to_vec()
    }

    fn altium_layer() -> Vec<u8> {
        text(
            "G04 #@! TF.GenerationSoftware,Altium Limited,Altium Designer,26.9.1*\n%FSLAX44Y44*%\n%MOMM*%\n%ADD10C,0.2000*%\nD10*\nX0Y0D02*\nX100000D01*\nM02*\n",
        )
    }

    fn altium_outline() -> Vec<u8> {
        text(
            "G04 #@! TF.GenerationSoftware,Altium Limited,Altium Designer,26.9.1*\n%FSLAX44Y44*%\n%MOMM*%\n%ADD10C,0.0500*%\nD10*\nX0Y0D02*\nX400000D01*\nY400000D01*\nX0D01*\nY0D01*\nM02*\n",
        )
    }

    fn altium_plane() -> Vec<u8> {
        text(
            "G04 #@! TF.GenerationSoftware,Altium Limited,Altium Designer,26.9.1*\n%FSLAX44Y44*%\n%MOMM*%\n%ADD88C,0.9000*%\nG04 #@! TF.FilePolarity,Negative*\nG36*\nX100000Y100000D02*\nX200000D01*\nY200000D01*\nX100000D01*\nY100000D01*\nD02*\nG37*\nD88*\nX150000Y150000D02*\nD03*\nM02*\n",
        )
    }

    fn altium_files() -> BTreeMap<String, Vec<u8>> {
        let mut files = BTreeMap::new();
        files.insert("main.GTL".to_string(), altium_layer());
        files.insert("main.GBL".to_string(), altium_layer());
        files.insert("main.GTS".to_string(), altium_layer());
        files.insert("main.GBS".to_string(), altium_layer());
        files.insert("main.GM".to_string(), altium_outline());
        files.insert("main.GP1".to_string(), altium_plane());
        files.insert("main.GP2".to_string(), altium_plane());
        files.insert(
            "main-RoundHoles.TXT".to_string(),
            text("M48\n;FILE_FORMAT=4:4\nMETRIC,LZ\n;TYPE=PLATED\nT01F00S00C0.3000\n;TYPE=NON_PLATED\nT04F00S00C0.9000\n%\nT01\nX0010000Y0010000\nT04\nX0020000Y0020000\nM30\n"),
        );
        files.insert(
            "main-SlotHoles.TXT".to_string(),
            text("M48\n;FILE_FORMAT=4:4\nMETRIC,LZ\n;TYPE=PLATED\nT02F00S00C0.5000\n%\nG90\nG05\nT02\nG00X0010000Y0010000\nM15\nG01X0020000Y0020000\nM16\nM30\n"),
        );
        files.insert(
            "Design Rule Check - main.html".to_string(),
            text("<html>report</html>"),
        );
        files.insert("binary.dat".to_string(), vec![0x00, 0x9f, 0x92, 0x96]);
        files
    }

    fn find<'a>(conversion: &'a Conversion, name: &str) -> &'a Output {
        conversion
            .outputs
            .iter()
            .find(|output| output.name == name)
            .unwrap_or_else(|| panic!("missing output {name}"))
    }

    #[test]
    fn converts_an_altium_board_with_planes() {
        let conversion = convert(altium_files(), &Settings::default()).expect("convert");

        assert_eq!(conversion.layers, 4);
        assert_eq!(conversion.mapping["main.GP1"], "Gerber_InnerLayer1.G1");
        assert_eq!(conversion.mapping["main.GP2"], "Gerber_InnerLayer2.G2");
        assert_eq!(
            conversion.mapping["main.GM"],
            "Gerber_BoardOutlineLayer.GKO"
        );

        let top =
            String::from_utf8(find(&conversion, "Gerber_TopLayer.GTL").data.clone()).expect("utf8");
        assert!(top.starts_with("G04 EasyEDA"));
        assert!(top.contains("G54D10*"));

        let drill = String::from_utf8(find(&conversion, "Drill_PTH_Through.DRL").data.clone())
            .expect("utf8");
        assert!(drill.contains("X10.00000Y10.00000"));
        assert!(drill.contains("G85"));

        let npth = String::from_utf8(find(&conversion, "Drill_NPTH_Through.DRL").data.clone())
            .expect("utf8");
        assert!(npth.contains("X20.00000Y20.00000"));

        assert!(
            conversion
                .warnings
                .iter()
                .any(|warning| warning.contains("negative plane")),
            "expected a negative-plane warning, got {:?}",
            conversion.warnings
        );
    }

    #[test]
    fn keeps_attachment_bytes_untouched() {
        let conversion = convert(altium_files(), &Settings::default()).expect("convert");
        let binary = find(&conversion, "binary.dat");
        assert_eq!(binary.data, vec![0x00, 0x9f, 0x92, 0x96]);
        assert!(!binary.plot);
    }

    #[test]
    fn clean_drops_reports_but_keeps_plots() {
        let settings = Settings {
            clean: true,
            ..Default::default()
        };
        let conversion = convert(altium_files(), &settings).expect("convert");

        assert!(
            conversion
                .outputs
                .iter()
                .all(|output| output.plot || output.name == GUIDE_NAME)
        );
        assert!(
            conversion
                .outputs
                .iter()
                .any(|output| output.name == "Gerber_TopLayer.GTL")
        );
        assert!(
            conversion
                .outputs
                .iter()
                .any(|output| output.name == "PCB下单必读.txt")
        );
    }

    #[test]
    fn converts_a_kicad_board_and_splits_drills() {
        let header = "G04 #@! TF.GenerationSoftware,KiCad,Pcbnew,8.0.1*\n%FSLAX46Y46*%\n%MOMM*%\n";
        let mut files = BTreeMap::new();
        for layer in [
            "F_Cu",
            "B_Cu",
            "F_Mask",
            "B_Mask",
            "F_Silkscreen",
            "B_Silkscreen",
        ] {
            files.insert(
                format!("board-{layer}.gbr"),
                text(&format!("{header}%ADD10C,0.2000*%\nD10*\nX0Y0D02*\nM02*\n")),
            );
        }
        files.insert(
            "board-Edge_Cuts.gbr".to_string(),
            text(&format!(
                "{header}G01*\nG75*\n%ADD10C,0.0500*%\nD10*\nX0Y0D02*\nX100000000D01*\nY100000000D01*\nX0D01*\nY0D01*\nM02*\n"
            )),
        );
        files.insert(
            "board-PTH.drl".to_string(),
            text("M48\n;DRILL file {KiCad 8.0.1}\nMETRIC\nT1C0.800\n%\nG90\nG05\nT1\nX10.500Y20.500\nM30\n"),
        );
        files.insert(
            "board-NPTH.drl".to_string(),
            text("M48\n;DRILL file {KiCad 8.0.1}\n; NPTH\nMETRIC\nT1C3.200\n%\nG90\nG05\nT1\nX5.000Y5.000\nM30\n"),
        );

        let conversion = convert(files, &Settings::default()).expect("convert");
        assert_eq!(conversion.layers, 2);
        assert_eq!(
            conversion.mapping["board-Edge_Cuts.gbr"],
            "Gerber_BoardOutlineLayer.GKO"
        );
        assert_eq!(conversion.mapping["board-F_Cu.gbr"], "Gerber_TopLayer.GTL");

        let pth = String::from_utf8(find(&conversion, "Drill_PTH_Through.DRL").data.clone())
            .expect("utf8");
        assert!(pth.contains("X10.50000Y20.50000"));

        let npth = String::from_utf8(find(&conversion, "Drill_NPTH_Through.DRL").data.clone())
            .expect("utf8");
        assert!(npth.contains("X5.00000Y5.00000"));
    }
}
