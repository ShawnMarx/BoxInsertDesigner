//! A Bambu Studio project 3MF: the print plates, each piece a named object lying where the plate
//! packer put it, and the X1C 0.4 / 0.20mm Standard process, so it opens ready to slice.
//!
//! The structure is copied from projects Bambu Studio saved (X1C projects saved in Studio; see
//! PLAN.md), not from Studio's source: meshes in `3D/Objects/object_<n>.model`, centred on their
//! bounding box; build items placing them on their plate; plates, names and the assembly view in
//! `Metadata/model_settings.config`; the process in `Metadata/project_settings.config`.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::plates::{self, M4, Piece, Plates};

/// Bambu Studio's settings for the X1C, 0.4 mm nozzle, 0.20mm Standard, with ten filament slots
/// (Bambu PLA Matte, Galaxy, Sparkle and Metal), from a project it saved (2.07.01, a
/// ten-filament build). A project's own settings are cut from it slot by slot (`settings`).
const X1C_SETTINGS: &str = include_str!("../assets/bambu/x1c-0.4-0.20-standard-10-filaments.json");
const TEMPLATE_SLOTS: usize = 10;

fn template() -> &'static serde_json::Map<String, serde_json::Value> {
    static T: std::sync::OnceLock<serde_json::Map<String, serde_json::Value>> = std::sync::OnceLock::new();
    T.get_or_init(|| serde_json::from_str(X1C_SETTINGS).expect("bundled Bambu settings"))
}

fn strings(v: &serde_json::Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().map(|x| x.as_str().unwrap_or_default().to_string()).collect()).unwrap_or_default()
}

/// Bambu's own colours for each preset: (name, #RRGGBB), from `assets/bambu/colours.json`. The
/// app offers these; a slot may still take any other colour.
pub fn colours() -> &'static serde_json::Map<String, serde_json::Value> {
    static C: std::sync::OnceLock<serde_json::Map<String, serde_json::Value>> = std::sync::OnceLock::new();
    C.get_or_init(|| {
        let mut m: serde_json::Map<String, serde_json::Value> = serde_json::from_str(include_str!("../assets/bambu/colours.json")).expect("bundled Bambu colours");
        m.retain(|k, _| !k.starts_with('_') && k != "unchecked");
        m
    })
}

/// The Bambu name of `color` in `preset`'s colours, if it is one of them.
pub fn colour_name(preset: &str, color: &str) -> Option<&'static str> {
    colours().get(preset)?.as_array()?.iter().find(|c| c[1].as_str().is_some_and(|h| h.eq_ignore_ascii_case(color)))?[0].as_str()
}

/// The filament presets the template has settings for.
pub fn presets() -> Vec<&'static str> {
    static P: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    let p = P.get_or_init(|| {
        let mut v = strings(&template()["filament_settings_id"]);
        let mut seen = std::collections::HashSet::new();
        v.retain(|x| seen.insert(x.clone()));
        v
    });
    p.iter().map(|s| s.as_str()).collect()
}

/// Project settings for these filament slots and `plates` plates: every per-filament setting
/// (one, two or four values a slot) taken from a template slot with the same preset, the flush
/// matrix cut to match, and the colours set. Cutting the template to its black Matte slot gives
/// a Studio-saved one-slot project back, but for `filament_colour_type` (checked 2026-10-28).
/// The project settings for `slots`, with each plate's purge tower where `towers` puts it
/// (the tower's own corner, inside its brim), or Studio's default.
pub fn settings(slots: &[crate::spec::Filament], towers: &[Option<[f64; 4]>]) -> Result<Vec<u8>, String> {
    let plates = towers.len();
    let t = template();
    let ids = strings(&t["filament_settings_id"]);
    let picks: Vec<usize> = slots
        .iter()
        .map(|f| ids.iter().position(|x| *x == f.preset).ok_or_else(|| format!("no settings for filament preset '{}'", f.preset)))
        .collect::<Result<_, _>>()?;
    let n = TEMPLATE_SLOTS;
    let mut o = t.clone();
    for (k, v) in t {
        let Some(a) = v.as_array() else { continue };
        let cut: Vec<serde_json::Value> = match k.as_str() {
            "flush_volumes_matrix" if a.len() == n * n => picks
                .iter()
                .enumerate()
                .flat_map(|(i, &pi)| picks.iter().enumerate().map(move |(j, &pj)| if i == j { serde_json::json!("0") } else if pi == pj { serde_json::json!("280") } else { a[pi * n + pj].clone() }))
                .collect(),
            "filament_self_index" => (0..slots.len()).flat_map(|i| std::iter::repeat_n(serde_json::json!((i + 1).to_string()), a.len() / n)).collect(),
            "wipe_tower_x" | "wipe_tower_y" => {
                let axis = if k == "wipe_tower_x" { 0 } else { 1 };
                let default = a.first().cloned().unwrap_or(serde_json::json!("165"));
                (0..plates.max(1))
                    .map(|i| match towers.get(i).copied().flatten() {
                        Some(r) => serde_json::json!(format!("{:.3}", r[axis] + crate::plates::TOWER_BRIM)),
                        None => default.clone(),
                    })
                    .collect()
            }
            _ if !a.is_empty() && a.len() % n == 0 => {
                let s = a.len() / n;
                picks.iter().flat_map(|&p| a[p * s..(p + 1) * s].iter().cloned()).collect()
            }
            _ => continue,
        };
        o.insert(k.clone(), serde_json::Value::Array(cut));
    }
    let colours: Vec<serde_json::Value> = slots.iter().map(|f| serde_json::json!(f.color.to_uppercase())).collect();
    o.insert("filament_colour".into(), serde_json::Value::Array(colours.clone()));
    o.insert("filament_multi_colour".into(), serde_json::Value::Array(colours));
    serde_json::to_vec_pretty(&o).map_err(|e| e.to_string())
}
/// The Studio version that saved it. Studio reads a 3MF as a project (plates and settings, not
/// just meshes) only when it names Studio as the application.
const STUDIO: &str = "02.07.01.62";

/// A part of a piece: its own mesh and filament slot (a label line on a tray).
struct PartMesh {
    name: String,
    filament: usize,
    verts: Vec<[f32; 3]>,
    tris: Vec<[u32; 3]>,
}

/// One piece, ready to write: its parts as they lie on the plate, centred on its bounding box.
struct Obj {
    name: String,
    plate: usize,
    filament: usize,
    parts: Vec<PartMesh>,
    /// The bounding box centre, plate coordinates.
    center: [f64; 3],
    /// Mesh (centred) → box coordinates, for Studio's assembly view.
    assembly: M4,
}

/// Days since 1970-01-01 → YYYY-MM-DD (UTC), by Hinnant's civil-from-days.
fn ymd(days: u64) -> String {
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// 3MF transforms act on row vectors: the rotation goes in transposed, then the translation.
fn transform(m: &M4) -> String {
    let v = [m[0][0], m[1][0], m[2][0], m[0][1], m[1][1], m[2][1], m[0][2], m[1][2], m[2][2], m[0][3], m[1][3], m[2][3]];
    v.iter().map(|x| num(*x)).collect::<Vec<_>>().join(" ")
}

fn num(x: f64) -> String {
    let r = (x * 1e6).round() / 1e6;
    format!("{}", r + 0.0)
}

/// A piece's parts: name, filament slot and print STL.
pub type PartFiles = Vec<(String, usize, PathBuf)>;

fn load(p: &Piece, files: &PartFiles, box_offset: [f64; 3]) -> Result<Obj, String> {
    let mut placed = vec![];
    for (name, filament, stl) in files {
        let tris = crate::geometry::read_stl(stl)?;
        placed.push((name, *filament, tris.iter().map(|v| plates::apply(&p.print_m, v.map(f64::from))).collect::<Vec<_>>()));
    }
    let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
    for v in placed.iter().flat_map(|x| &x.2) {
        for i in 0..3 {
            lo[i] = lo[i].min(v[i]);
            hi[i] = hi[i].max(v[i]);
        }
    }
    let center = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0];
    let mut parts = vec![];
    for (name, filament, on_plate) in placed {
        // Shared vertices, as an indexed mesh.
        let mut index: HashMap<[i64; 3], u32> = HashMap::new();
        let mut verts = vec![];
        let mut faces = vec![];
        for t in on_plate.chunks(3) {
            let mut f = [0u32; 3];
            for (k, v) in t.iter().enumerate() {
                let c = [v[0] - center[0], v[1] - center[1], v[2] - center[2]];
                let key = c.map(|x| (x * 1e4).round() as i64);
                f[k] = *index.entry(key).or_insert_with(|| {
                    verts.push(c.map(|x| x as f32));
                    (verts.len() - 1) as u32
                });
            }
            if f[0] != f[1] && f[1] != f[2] && f[0] != f[2] {
                faces.push(f);
            }
        }
        parts.push(PartMesh { name: name.clone(), filament, verts, tris: faces });
    }
    // mesh → plate → print → view (box coordinates), then the box's place in the assembly.
    let to_print = plates::invert(&p.print_m);
    let to_view = plates::invert(&p.view_to_print);
    let assembly = plates::mul(&plates::translate(box_offset), &plates::mul(&to_view, &plates::mul(&to_print, &plates::translate(center))));
    Ok(Obj { name: p.name.clone(), plate: p.plate, filament: p.filament, parts, center, assembly })
}

/// A piece's object file: one mesh object per part, ids `first`, `first + 1`, …
fn object_model(first: usize, o: &Obj) -> String {
    let mut s = String::with_capacity(o.parts.iter().map(|p| p.verts.len() * 48 + p.tris.len() * 40).sum::<usize>() + 1024);
    s.push_str(r#"<?xml version="1.0" encoding="UTF-8"?>
<model unit="millimeter" xml:lang="en-US" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02" xmlns:BambuStudio="http://schemas.bambulab.com/package/2021" xmlns:p="http://schemas.microsoft.com/3dmanufacturing/production/2015/06" requiredextensions="p">
 <metadata name="BambuStudio:3mfVersion">1</metadata>
 <resources>
"#);
    for (k, part) in o.parts.iter().enumerate() {
        let id = first + k;
        s.push_str(&format!("  <object id=\"{id}\" p:UUID=\"{}\" type=\"model\">\n   <mesh>\n    <vertices>\n", uuid(id as u32, 1)));
        for v in &part.verts {
            s.push_str(&format!("     <vertex x=\"{}\" y=\"{}\" z=\"{}\"/>\n", v[0], v[1], v[2]));
        }
        s.push_str("    </vertices>\n    <triangles>\n");
        for t in &part.tris {
            s.push_str(&format!("     <triangle v1=\"{}\" v2=\"{}\" v3=\"{}\"/>\n", t[0], t[1], t[2]));
        }
        s.push_str("    </triangles>\n   </mesh>\n  </object>\n");
    }
    s.push_str(" </resources>\n <build/>\n</model>\n");
    s
}

/// A UUID-shaped id, stable for a given object: `kind` keeps objects, components and items apart.
fn uuid(n: u32, kind: u32) -> String {
    format!("{n:08x}-b1d0-4{kind:03x}-8000-000000000000")
}

/// Write `pl` as a Bambu Studio project at `path`, with these filament slots. `parts_of` gives
/// each piece's parts (name, slot, print STL); boxes sit side by side in the assembly view.
pub fn write_project(path: &Path, title: &str, pl: &Plates, slots: &[crate::spec::Filament], parts_of: &dyn Fn(&Piece) -> PartFiles) -> Result<(), String> {
    let mut boxes: Vec<&str> = vec![];
    for p in &pl.pieces {
        if !boxes.contains(&p.box_.as_str()) {
            boxes.push(&p.box_);
        }
    }
    let mut objs = vec![];
    for p in &pl.pieces {
        let k = boxes.iter().position(|b| *b == p.box_).unwrap_or(0);
        objs.push(load(p, &parts_of(p), [k as f64 * 400.0, 0.0, 0.0])?);
    }
    let settings_json = settings(slots, &pl.towers)?;
    let today = ymd(crate::now_ms() / 86_400_000);
    // Ids as Studio numbers them: a piece's part meshes first, then the object wrapping them.
    let mut ids = vec![];
    let mut next = 1;
    for o in &objs {
        ids.push((next, next + o.parts.len()));
        next += o.parts.len() + 1;
    }

    let mut model = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<model unit="millimeter" xml:lang="en-US" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02" xmlns:BambuStudio="http://schemas.bambulab.com/package/2021" xmlns:p="http://schemas.microsoft.com/3dmanufacturing/production/2015/06" requiredextensions="p">
 <metadata name="Application">BambuStudio-{STUDIO}</metadata>
 <metadata name="BambuStudio:3mfVersion">1</metadata>
 <metadata name="CreationDate">{today}</metadata>
 <metadata name="Description">Generated by BoxInsertDesigner</metadata>
 <metadata name="ModificationDate">{today}</metadata>
 <metadata name="Title">{}</metadata>
 <resources>
"#,
        esc(title)
    );
    let mut rels = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\n");
    for (i, o) in objs.iter().enumerate() {
        let n = i + 1;
        let (first, wrap) = ids[i];
        model.push_str(&format!("  <object id=\"{wrap}\" p:UUID=\"{}\" type=\"model\">\n   <components>\n", uuid(n as u32, 2)));
        for k in 0..o.parts.len() {
            model.push_str(&format!(
                "    <component p:path=\"/3D/Objects/object_{n}.model\" objectid=\"{}\" p:UUID=\"{}\" transform=\"1 0 0 0 1 0 0 0 1 0 0 0\"/>\n",
                first + k,
                uuid((first + k) as u32, 3)
            ));
        }
        model.push_str("   </components>\n  </object>\n");
        rels.push_str(&format!(" <Relationship Target=\"/3D/Objects/object_{n}.model\" Id=\"rel-{n}\" Type=\"http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel\"/>\n"));
    }
    rels.push_str("</Relationships>\n");
    model.push_str(&format!(" </resources>\n <build p:UUID=\"{}\">\n", uuid(0, 4)));
    for (i, o) in objs.iter().enumerate() {
        let [ox, oy] = pl.plate_origin(o.plate);
        let place = plates::translate([o.center[0] + ox, o.center[1] + oy, o.center[2]]);
        model.push_str(&format!("  <item objectid=\"{}\" p:UUID=\"{}\" transform=\"{}\" printable=\"1\"/>\n", ids[i].1, uuid(i as u32 + 1, 5), transform(&place)));
    }
    model.push_str(" </build>\n</model>\n");

    let mut settings = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<config>\n");
    for (i, o) in objs.iter().enumerate() {
        let (first, wrap) = ids[i];
        let faces: usize = o.parts.iter().map(|p| p.tris.len()).sum();
        settings.push_str(&format!(
            "  <object id=\"{wrap}\">\n    <metadata key=\"name\" value=\"{}\"/>\n    <metadata key=\"extruder\" value=\"{}\"/>\n    <metadata face_count=\"{faces}\"/>\n",
            esc(&o.name),
            o.filament
        ));
        for (k, part) in o.parts.iter().enumerate() {
            settings.push_str(&format!(
                "    <part id=\"{}\" subtype=\"normal_part\">\n      <metadata key=\"name\" value=\"{}\"/>\n      <metadata key=\"matrix\" value=\"1 0 0 0 0 1 0 0 0 0 1 0 0 0 0 1\"/>\n      <metadata key=\"extruder\" value=\"{}\"/>\n      <mesh_stat face_count=\"{}\" edges_fixed=\"0\" degenerate_facets=\"0\" facets_removed=\"0\" facets_reversed=\"0\" backwards_edges=\"0\"/>\n    </part>\n",
                first + k,
                esc(&part.name),
                part.filament,
                part.tris.len()
            ));
        }
        settings.push_str("  </object>\n");
    }
    let maps = vec!["1"; slots.len()].join(" ");
    let vols = vec!["0"; slots.len()].join(" ");
    for plate in 1..=pl.count {
        let name = pl.names.get(plate - 1).map(String::as_str).unwrap_or("");
        settings.push_str(&format!(
            "  <plate>\n    <metadata key=\"plater_id\" value=\"{plate}\"/>\n    <metadata key=\"plater_name\" value=\"{}\"/>\n    <metadata key=\"locked\" value=\"false\"/>\n    <metadata key=\"filament_map_mode\" value=\"Auto For Flush\"/>\n    <metadata key=\"filament_maps\" value=\"{maps}\"/>\n    <metadata key=\"filament_volume_maps\" value=\"{vols}\"/>\n",
            esc(name)
        ));
        for (i, _) in objs.iter().enumerate().filter(|(_, o)| o.plate == plate) {
            settings.push_str(&format!(
                "    <model_instance>\n      <metadata key=\"object_id\" value=\"{}\"/>\n      <metadata key=\"instance_id\" value=\"0\"/>\n      <metadata key=\"identify_id\" value=\"{}\"/>\n    </model_instance>\n",
                ids[i].1,
                100 + i
            ));
        }
        settings.push_str("  </plate>\n");
    }
    settings.push_str("  <assemble>\n");
    for (i, o) in objs.iter().enumerate() {
        settings.push_str(&format!("   <assemble_item object_id=\"{}\" instance_id=\"0\" transform=\"{}\" offset=\"0 0 0\" />\n", ids[i].1, transform(&o.assembly)));
    }
    settings.push_str("  </assemble>\n</config>\n");

    let mut cut = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<objects>\n");
    for n in 1..=objs.len() {
        cut.push_str(&format!(" <object id=\"{n}\">\n  <cut_id id=\"0\" check_sum=\"1\" connectors_cnt=\"0\"/>\n </object>\n"));
    }
    cut.push_str("</objects>\n");
    let seq: serde_json::Map<String, serde_json::Value> =
        (1..=pl.count).map(|n| (format!("plate_{n}"), serde_json::json!({ "nozzle_sequence": [], "optimal_assignment": [], "sequence": [] }))).collect();

    let mut files: Vec<(String, Vec<u8>)> = vec![
        ("[Content_Types].xml".into(), CONTENT_TYPES.into()),
        ("_rels/.rels".into(), ROOT_RELS.into()),
        ("3D/3dmodel.model".into(), model.into_bytes()),
        ("3D/_rels/3dmodel.model.rels".into(), rels.into_bytes()),
    ];
    for (i, o) in objs.iter().enumerate() {
        files.push((format!("3D/Objects/object_{}.model", i + 1), object_model(ids[i].0, o).into_bytes()));
    }
    files.extend([
        ("Metadata/project_settings.config".into(), settings_json),
        ("Metadata/model_settings.config".into(), settings.into_bytes()),
        ("Metadata/cut_information.xml".into(), cut.into_bytes()),
        ("Metadata/slice_info.config".into(), format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<config>\n  <header>\n    <header_item key=\"X-BBL-Client-Type\" value=\"slicer\"/>\n    <header_item key=\"X-BBL-Client-Version\" value=\"{STUDIO}\"/>\n  </header>\n</config>\n").into_bytes()),
        ("Metadata/filament_sequence.json".into(), serde_json::to_vec(&seq).unwrap()),
    ]);

    let f = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut z = zip::ZipWriter::new(f);
    let opt = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, bytes) in files {
        z.start_file(name, opt).map_err(|e| e.to_string())?;
        z.write_all(&bytes).map_err(|e| e.to_string())?;
    }
    z.finish().map_err(|e| e.to_string())?;
    Ok(())
}

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
 <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
 <Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>
 <Default Extension="png" ContentType="image/png"/>
 <Default Extension="gcode" ContentType="text/x.gcode"/>
</Types>
"#;

const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
 <Relationship Target="/3D/3dmodel.model" Id="rel-1" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/>
</Relationships>
"#;

#[cfg(test)]
mod tests {
    #[test]
    fn settings_cut_to_the_slots() {
        use crate::spec::Filament;
        let slots = [Filament { preset: "Bambu PLA Matte @BBL X1C".into(), color: "#000000".into() }, Filament { preset: "Bambu PLA Galaxy @BBL X1C".into(), color: "#594177".into() }];
        let v: serde_json::Value = serde_json::from_slice(&super::settings(&slots, &[None, Some([200.0, 205.0, 241.0, 251.0]), None]).unwrap()).unwrap();
        assert_eq!(v["filament_settings_id"], serde_json::json!(["Bambu PLA Matte @BBL X1C", "Bambu PLA Galaxy @BBL X1C"]));
        assert_eq!(v["filament_colour"], serde_json::json!(["#000000", "#594177"]));
        assert_eq!(v["flush_volumes_matrix"].as_array().unwrap().len(), 4);
        assert_eq!(v["nozzle_temperature"].as_array().unwrap().len(), 4);
        assert_eq!(v["filament_self_index"], serde_json::json!(["1", "1", "2", "2"]));
        assert_eq!(v["wipe_tower_x"].as_array().unwrap().len(), 3);
        assert_eq!(v["wipe_tower_x"][1], serde_json::json!("203.000"));
        assert_eq!(v["wipe_tower_y"][1], serde_json::json!("208.000"));
        assert!(super::settings(&[Filament { preset: "Nope".into(), color: "#000000".into() }], &[None]).is_err());
    }

    #[test]
    fn dates() {
        assert_eq!(super::ymd(0), "1970-01-01");
        assert_eq!(super::ymd(20_754), "2026-10-28");
    }
}

#[cfg(test)]
mod colour_tests {
    use super::*;

    #[test]
    fn every_preset_has_bambu_colours() {
        for p in presets() {
            let c = colours().get(p).and_then(|v| v.as_array()).unwrap_or_else(|| panic!("no colours for {p}"));
            assert!(!c.is_empty());
            for x in c {
                let h = x[1].as_str().unwrap();
                assert!(h.len() == 7 && h.starts_with('#') && h[1..].chars().all(|c| c.is_ascii_hexdigit()), "{p}: {h}");
            }
        }
        assert_eq!(colour_name("Bambu PLA Matte @BBL X1C", "#9b9ea0"), Some("Ash Gray"));
        assert_eq!(colour_name("Bambu PLA Matte @BBL X1C", "#123456"), None);
    }
}
