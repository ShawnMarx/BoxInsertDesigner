//! Pieces brought in as they are, from a Bambu Studio project 3MF: each object becomes a piece
//! whose parts (a tray body, its label lines) keep their names and filament slots, stored as
//! STLs in `games/<slug>/pieces/<id>/` in print orientation. The project's plates, plate names,
//! positions on the plates and filament slots come with them. Nothing here is regenerated: the
//! packer doesn't make these, so where each copy sits in the box is set explicitly
//! (`place_piece`).

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::Reason;
use crate::plates::{self, M4};
use crate::spec::{Filament, Piece, PieceCopy, PiecePart, PlatePin, Spec};

pub fn dir(games: &Path, slug: &str, piece: &str) -> PathBuf {
    games.join(slug).join("pieces").join(piece)
}

/// What an import brings: the pieces, their places on the plates, plate names and filaments.
pub struct Imported {
    pub pieces: Vec<Piece>,
    pub pins: Vec<PlatePin>,
    pub plate_names: Vec<String>,
    pub filaments: Vec<Filament>,
}

/// A 3MF transform (row-vector, 12 numbers) as a column-vector matrix.
fn parse_transform(s: Option<&str>) -> M4 {
    let v: Vec<f64> = s.unwrap_or("").split_whitespace().filter_map(|x| x.parse().ok()).collect();
    if v.len() != 12 {
        return plates::identity();
    }
    let mut m = plates::identity();
    for i in 0..3 {
        for j in 0..3 {
            m[i][j] = v[j * 3 + i];
        }
        m[i][3] = v[9 + i];
    }
    m
}

fn piece_id(name: &str) -> String {
    let s = crate::commands::slugify(name.trim_end_matches(".stl").trim_end_matches(".STL"));
    if s.is_empty() { "piece".into() } else { s }
}

fn write_stl(path: &Path, tris: &[[f64; 3]]) -> Result<(), String> {
    let mut b = vec![0u8; 80];
    b.extend((tris.len() as u32 / 3).to_le_bytes());
    for t in tris.chunks(3) {
        b.extend([0u8; 12]);
        for p in t {
            for c in p {
                b.extend((*c as f32).to_le_bytes());
            }
        }
        b.extend([0u8; 2]);
    }
    std::fs::write(path, b).map_err(|e| format!("{}: {e}", path.display()))
}

/// The triangles of every mesh object in a model file, by object id.
fn meshes(xml: &str) -> Result<HashMap<String, Vec<[f64; 3]>>, String> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| e.to_string())?;
    let mut out = HashMap::new();
    for o in doc.descendants().filter(|n| n.has_tag_name("object")) {
        let Some(mesh) = o.children().find(|n| n.has_tag_name("mesh")) else { continue };
        let num = |n: &roxmltree::Node, a: &str| n.attribute(a).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
        let verts: Vec<[f64; 3]> = mesh
            .descendants()
            .filter(|n| n.has_tag_name("vertex"))
            .map(|v| [num(&v, "x"), num(&v, "y"), num(&v, "z")])
            .collect();
        let mut tris = vec![];
        for t in mesh.descendants().filter(|n| n.has_tag_name("triangle")) {
            for a in ["v1", "v2", "v3"] {
                let i: usize = t.attribute(a).and_then(|v| v.parse().ok()).ok_or("bad triangle")?;
                tris.push(*verts.get(i).ok_or("triangle past the vertices")?);
            }
        }
        out.insert(o.attribute("id").unwrap_or_default().to_string(), tris);
    }
    Ok(out)
}

/// Read a Bambu Studio project and write its objects' parts under `games/<slug>/pieces/`.
pub fn import(games: &Path, slug: &str, bytes: &[u8], file: &str, taken: &[String]) -> Result<Imported, Reason> {
    let bad = |m: String| Reason::new("bad_3mf", m);
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| bad(e.to_string()))?;
    let mut read = |name: &str| -> Result<String, Reason> {
        let mut s = String::new();
        zip.by_name(name).map_err(|e| bad(format!("{name}: {e}")))?.read_to_string(&mut s).map_err(|e| bad(e.to_string()))?;
        Ok(s)
    };
    let model = read("3D/3dmodel.model")?;
    let settings = read("Metadata/model_settings.config")?;
    let project: serde_json::Value = serde_json::from_str(&read("Metadata/project_settings.config")?).map_err(|e| bad(e.to_string()))?;
    let mdoc = roxmltree::Document::parse(&model).map_err(|e| bad(e.to_string()))?;
    let sdoc = roxmltree::Document::parse(&settings).map_err(|e| bad(e.to_string()))?;
    let meta = |n: roxmltree::Node, key: &str| -> Option<String> {
        n.children().find(|c| c.has_tag_name("metadata") && c.attribute("key") == Some(key)).and_then(|c| c.attribute("value")).map(String::from)
    };

    // Filament slots: the presets and colours the project uses.
    let strs = |k: &str| -> Vec<String> { project[k].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default() };
    let filaments: Vec<Filament> = strs("filament_settings_id").into_iter().zip(strs("filament_colour")).map(|(preset, color)| Filament { preset, color: color.to_uppercase() }).collect();

    // Plates: names, and which objects are on each.
    let mut plate_of: HashMap<String, usize> = HashMap::new();
    let mut plate_names = vec![];
    for p in sdoc.descendants().filter(|n| n.has_tag_name("plate")) {
        let id: usize = meta(p, "plater_id").and_then(|v| v.parse().ok()).unwrap_or(plate_names.len() + 1);
        if plate_names.len() < id {
            plate_names.resize(id, String::new());
        }
        plate_names[id - 1] = meta(p, "plater_name").unwrap_or_default();
        for mi in p.children().filter(|c| c.has_tag_name("model_instance")) {
            if let Some(o) = meta(mi, "object_id") {
                plate_of.insert(o, id);
            }
        }
    }
    let count = plate_names.len();
    let cols = (count as f64).sqrt().ceil().max(1.0) as usize;
    let origin = |n: usize| [((n - 1) % cols) as f64 * 256.0 * plates::PLATE_STRIDE, -(((n - 1) / cols) as f64) * 256.0 * plates::PLATE_STRIDE];

    // The objects' names, slots and parts.
    struct Info {
        name: String,
        extruder: usize,
        parts: HashMap<String, (String, Option<usize>)>,
    }
    let mut info: HashMap<String, Info> = HashMap::new();
    for o in sdoc.root_element().children().filter(|n| n.has_tag_name("object")) {
        let parts = o
            .children()
            .filter(|c| c.has_tag_name("part"))
            .map(|p| (p.attribute("id").unwrap_or_default().to_string(), (meta(p, "name").unwrap_or_default(), meta(p, "extruder").and_then(|v| v.parse().ok()))))
            .collect();
        info.insert(
            o.attribute("id").unwrap_or_default().to_string(),
            Info { name: meta(o, "name").unwrap_or_default(), extruder: meta(o, "extruder").and_then(|v| v.parse().ok()).unwrap_or(1), parts },
        );
    }

    let mut files: HashMap<String, HashMap<String, Vec<[f64; 3]>>> = HashMap::new();
    let objects: HashMap<&str, roxmltree::Node> =
        mdoc.descendants().filter(|n| n.has_tag_name("object")).filter_map(|n| n.attribute("id").map(|id| (id, n))).collect();
    let mut out = Imported { pieces: vec![], pins: vec![], plate_names, filaments };
    let mut used: Vec<String> = taken.to_vec();
    for item in mdoc.descendants().filter(|n| n.has_tag_name("item")) {
        let oid = item.attribute("objectid").unwrap_or_default();
        let Some(obj) = objects.get(oid) else { continue };
        let inf = info.get(oid);
        let name = inf.map(|i| i.name.trim_end_matches(".stl").to_string()).unwrap_or_else(|| format!("Object {oid}"));
        let place = parse_transform(item.attribute("transform"));
        // Turn it as it lies on its plate, but leave the moving to the plate position.
        let mut turn = place;
        for r in turn.iter_mut().take(3) {
            r[3] = 0.0;
        }
        let mut parts: Vec<(String, Option<usize>, Vec<[f64; 3]>)> = vec![];
        for c in obj.descendants().filter(|n| n.has_tag_name("component")) {
            let path = c.attributes().find(|a| a.name() == "path").map(|a| a.value().trim_start_matches('/').to_string()).unwrap_or_else(|| "3D/3dmodel.model".into());
            if !files.contains_key(&path) {
                let xml = read(&path)?;
                files.insert(path.clone(), meshes(&xml).map_err(|e| bad(format!("{path}: {e}")))?);
            }
            let cid = c.attribute("objectid").unwrap_or_default();
            let Some(tris) = files[&path].get(cid) else { continue };
            let m = plates::mul(&turn, &parse_transform(c.attribute("transform")));
            let (pname, ext) = inf.and_then(|i| i.parts.get(cid).cloned()).unwrap_or_else(|| (name.clone(), None));
            parts.push((pname, ext, tris.iter().map(|p| plates::apply(&m, *p)).collect()));
        }
        if parts.is_empty() {
            continue;
        }
        let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
        for p in parts.iter().flat_map(|x| &x.2) {
            for i in 0..3 {
                lo[i] = lo[i].min(p[i]);
                hi[i] = hi[i].max(p[i]);
            }
        }
        let mut id = piece_id(&name);
        let base = id.clone();
        let mut k = 2;
        while used.contains(&id) {
            id = format!("{base}-{k}");
            k += 1;
        }
        used.push(id.clone());
        let d = dir(games, slug, &id);
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).map_err(|e| Reason::new("io", e.to_string()))?;
        let mut sparts = vec![];
        let filament = inf.map_or(1, |i| i.extruder);
        // Part names repeat ("Label", "Label"): number the files, keep the names.
        for (n, (pname, ext, tris)) in parts.into_iter().enumerate() {
            let file = format!("{n}.stl");
            let moved: Vec<[f64; 3]> = tris.iter().map(|p| [p[0] - lo[0], p[1] - lo[1], p[2] - lo[2]]).collect();
            write_stl(&d.join(&file), &moved).map_err(|e| Reason::new("io", e))?;
            sparts.push(PiecePart { name: pname, file, filament: ext.filter(|e| *e != filament) });
        }
        let size = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
        let found_in = |p: &Piece| detect_pockets(games, slug, p).unwrap_or_default();
        if let Some(&plate) = plate_of.get(oid) {
            let o = origin(plate);
            let at = [place[0][3] + lo[0] - o[0], place[1][3] + lo[1] - o[1]];
            out.pins.push(PlatePin { piece: format!("piece/{id}"), plate: Some(plate), pos: Some(at), ..Default::default() });
        }
        out.pieces.push(Piece {
            id,
            name,
            source: Some(format!("{file}: object {oid}")),
            size,
            filament,
            parts: sparts,
            box_: None,
            auto_name: None,
            request: None,
            log: vec![],
            pockets: vec![],
            copies: vec![PieceCopy::default()],
            design: None,
        });
        let last = out.pieces.last_mut().unwrap();
        last.pockets = found_in(last);
    }
    Ok(out)
}

/// The file a piece made in SolidCraft keeps its design in, beside its parts.
pub const DESIGN: &str = "design.sc";

/// A piece's parts from a SolidCraft design (`import_design`): one per body, named as the body,
/// as it lies in the design (z up, as it prints), moved to stand at the origin. The design is
/// kept beside them (`design.sc`) so it can be opened again. Returns the parts and the size.
pub async fn from_design(cli: &Path, games: &Path, slug: &str, id: &str, design: &[u8]) -> Result<(Vec<PiecePart>, [f64; 3]), Reason> {
    if !design.starts_with(b"PK") {
        return Err(Reason::new("bad_design", "not a SolidCraft design (.sc): expected a zip container"));
    }
    let io = |e: std::io::Error| Reason::new("io", e.to_string());
    let d = dir(games, slug, id);
    let work = d.with_extension(format!("new-{}", crate::now_ms()));
    std::fs::create_dir_all(&work).map_err(io)?;
    let result = async {
        std::fs::write(work.join(DESIGN), design).map_err(io)?;
        crate::edit::cli(cli, &work, &["eval", DESIGN, "--json-out", "eval.json"]).await?;
        let bodies: Vec<String> = crate::edit::read_json(&work.join("eval.json"))?["bodies"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|b| b["name"].as_str().map(String::from))
            .collect();
        if bodies.is_empty() {
            return Err(Reason::new("empty_design", "the design has no solid bodies to print"));
        }
        let saves: Vec<serde_json::Value> = bodies
            .iter()
            .enumerate()
            .map(|(n, b)| serde_json::json!({ "command": "file.save_mesh", "params": { "path": work.join(format!("raw{n}.stl")).to_string_lossy(), "bodies": [b] } }))
            .collect();
        std::fs::write(work.join("parts.json"), serde_json::to_vec(&serde_json::json!({ "commands": saves })).unwrap()).map_err(io)?;
        crate::edit::cli(cli, &work, &["run", "parts.json", "--in", DESIGN, "--quiet"]).await?;
        let mut meshes = vec![];
        for n in 0..bodies.len() {
            let tris = crate::geometry::read_stl(&work.join(format!("raw{n}.stl"))).map_err(|e| Reason::new("io", e))?;
            meshes.push(tris);
        }
        let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
        for p in meshes.iter().flatten() {
            for i in 0..3 {
                lo[i] = lo[i].min(p[i] as f64);
                hi[i] = hi[i].max(p[i] as f64);
            }
        }
        let mut parts = vec![];
        for (n, (tris, name)) in meshes.iter().zip(&bodies).enumerate() {
            let file = format!("{n}.stl");
            let moved: Vec<[f64; 3]> = tris.iter().map(|p| [p[0] as f64 - lo[0], p[1] as f64 - lo[1], p[2] as f64 - lo[2]]).collect();
            write_stl(&work.join(&file), &moved).map_err(|e| Reason::new("io", e))?;
            let _ = std::fs::remove_file(work.join(format!("raw{n}.stl")));
            parts.push(PiecePart { name: name.clone(), file, filament: None });
        }
        for f in ["eval.json", "parts.json"] {
            let _ = std::fs::remove_file(work.join(f));
        }
        Ok((parts, [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]]))
    }
    .await;
    match result {
        Ok(r) => {
            let _ = std::fs::remove_dir_all(&d);
            std::fs::rename(&work, &d).map_err(io)?;
            Ok(r)
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(&work);
            Err(e)
        }
    }
}

/// A copy's place in the box: print-local → box coordinates, following `inside` up through its
/// parents. None when it (or a parent) isn't placed.
pub fn copy_to_box(s: &Spec, piece: &Piece, copy: &PieceCopy) -> Option<M4> {
    copy_to_box_depth(s, piece, copy, 0)
}

fn copy_to_box_depth(s: &Spec, piece: &Piece, copy: &PieceCopy, depth: usize) -> Option<M4> {
    if depth > 8 {
        return None;
    }
    let at = copy.at?;
    let mut o = if copy.flip {
        let mut m = plates::identity();
        m[1][1] = -1.0;
        m[2][2] = -1.0;
        m
    } else {
        plates::identity()
    };
    o = plates::mul(&rot_x(copy.tilt), &o);
    o = plates::mul(&rot_z(copy.turn), &o);
    // Its min corner after turning goes to `at`.
    let (mut lo, mut first) = ([0.0; 3], true);
    for k in 0..8 {
        let c = [if k & 1 == 0 { 0.0 } else { piece.size[0] }, if k & 2 == 0 { 0.0 } else { piece.size[1] }, if k & 4 == 0 { 0.0 } else { piece.size[2] }];
        let p = plates::apply(&o, c);
        for i in 0..3 {
            lo[i] = if first { p[i] } else { lo[i].min(p[i]) };
        }
        first = false;
    }
    let local = plates::mul(&plates::translate([at[0] - lo[0], at[1] - lo[1], at[2] - lo[2]]), &o);
    match &copy.inside {
        None => Some(local),
        Some(parent) => {
            let (pid, cname) = parent.split_once('/').map_or((parent.as_str(), None), |(a, b)| (a, Some(b)));
            let pp = s.pieces.iter().find(|p| p.id == pid)?;
            let pc = match cname {
                Some(n) => pp.copies.iter().find(|c| c.name.as_deref() == Some(n))?,
                None => pp.copies.first()?,
            };
            // Inside: in the parent's own frame as it sits, its min corner at the origin.
            let pm = copy_to_box_depth(s, pp, pc, depth + 1)?;
            let (plo, _) = bounds(&pm, pp.size);
            Some(plates::mul(&plates::translate(plo), &local))
        }
    }
}

fn bounds(m: &M4, size: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let (mut a, mut b) = ([f64::MAX; 3], [f64::MIN; 3]);
    for k in 0..8 {
        let c = [if k & 1 == 0 { 0.0 } else { size[0] }, if k & 2 == 0 { 0.0 } else { size[1] }, if k & 4 == 0 { 0.0 } else { size[2] }];
        let p = plates::apply(m, c);
        for i in 0..3 {
            a[i] = a[i].min(p[i]);
            b[i] = b[i].max(p[i]);
        }
    }
    (a, b)
}

/// Where a placed copy sits in the box: min and max corners.
pub fn copy_bounds(s: &Spec, piece: &Piece, copy: &PieceCopy) -> Option<([f64; 3], [f64; 3])> {
    copy_to_box(s, piece, copy).map(|m| bounds(&m, piece.size))
}

/// Quarter turns about x: 90 takes y up to z.
fn rot_x(deg: u32) -> M4 {
    let (c, s) = match deg % 360 {
        90 => (0.0, 1.0),
        180 => (-1.0, 0.0),
        270 => (0.0, -1.0),
        _ => (1.0, 0.0),
    };
    let mut m = plates::identity();
    (m[1][1], m[1][2], m[2][1], m[2][2]) = (c, -s, s, c);
    m
}

fn rot_z(deg: u32) -> M4 {
    let (c, s) = match deg % 360 {
        90 => (0.0, 1.0),
        180 => (-1.0, 0.0),
        270 => (0.0, -1.0),
        _ => (1.0, 0.0),
    };
    let mut m = plates::identity();
    (m[0][0], m[0][1], m[1][0], m[1][1]) = (c, -s, s, c);
    m
}

/// Write a part's mesh moved by `m` (into the box, for the viewer).
pub fn write_moved(src: &Path, m: &M4, out: &Path) -> Result<(), String> {
    let tris: Vec<[f64; 3]> = crate::geometry::read_stl(src)?.into_iter().map(|p| plates::apply(m, p.map(f64::from))).collect();
    write_stl(out, &tris)
}

/// The box `box_id`'s placed copies, as the viewer's meshes: each part moved into the box and
/// written under `out`, in its filament's colour.
pub fn view_meshes(s: &Spec, games: &Path, slug: &str, box_id: &str, out: &Path) -> Result<Vec<crate::geometry::MeshIn>, String> {
    let first = s.boxes().first().map(|b| b.id.clone()).unwrap_or_default();
    let slots = s.filament_slots();
    let colour = |f: usize| slots.get(f.saturating_sub(1)).map_or("#9b9ea0".to_string(), |x| x.color.clone());
    let mut meshes = vec![];
    for p in s.pieces.iter().filter(|p| p.box_.as_deref().unwrap_or(&first) == box_id) {
        for (ci, c) in p.copies.iter().enumerate() {
            let Some(m) = copy_to_box(s, p, c) else { continue };
            std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
            let filament = c.filament.unwrap_or(p.filament);
            for (k, part) in p.parts.iter().enumerate() {
                let stl = out.join(format!("{}-{ci}-{k}.stl", p.id));
                write_moved(&dir(games, slug, &p.id).join(&part.file), &m, &stl)?;
                // A copy in its own colour recolours the body; parts with a slot of their own keep it.
                let f = part.filament.unwrap_or(filament);
                meshes.push(crate::geometry::MeshIn {
                    name: format!("{}-{ci}-{k}", p.id),
                    stl,
                    color: colour(f),
                    opacity: 1.0,
                    // A tray to the viewer: selection, the Tray tab, Motion, Measure and Cut all work.
                    extras: serde_json::json!({
                        "kind": "tray", "tray": tray_id(p, c), "piece": p.id, "copy": c.name, "part": part.name, "name": p.name,
                        "layer": c.layer.unwrap_or(1).saturating_sub(1), "filament": f,
                    }),
                });
            }
        }
    }
    Ok(meshes)
}

/// The id a placed copy goes by as a tray in the app: the piece's, or `<piece>.<copy>` for one of
/// several.
pub fn tray_id(p: &Piece, c: &PieceCopy) -> String {
    match (&c.name, p.copies.len() > 1) {
        (Some(n), true) => format!("{}.{}", p.id, crate::commands::slugify(n)),
        _ => p.id.clone(),
    }
}

/// The box's placed copies as trays for the app (`imported: true`): place, size, guide layer,
/// colour, and what they hold (components kept in the piece) as pockets.
pub fn trays(s: &Spec, box_id: &str) -> Vec<serde_json::Value> {
    use serde_json::json;
    let first = s.boxes().first().map(|b| b.id.clone()).unwrap_or_default();
    let slots = s.filament_slots();
    let colour = |f: usize| slots.get(f.saturating_sub(1)).map_or("#9b9ea0".to_string(), |x| x.color.clone());
    let mut out = vec![];
    for p in s.pieces.iter().filter(|p| p.box_.as_deref().unwrap_or(&first) == box_id) {
        let held: Vec<serde_json::Value> = s
            .components
            .iter()
            .filter(|c| c.piece.as_deref() == Some(p.id.as_str()))
            .map(|c| {
                // The count as stored (cards, tokens, minis), else as measured.
                let kind = serde_json::to_value(&c.kind).unwrap_or_default();
                let n = kind["count"].as_u64().or_else(|| c.measure.as_ref().and_then(|m| m.count).map(u64::from)).unwrap_or(1);
                json!({
                    "id": format!("{}:{}", p.id, c.id), "label": c.name, "component": c.id, "items": n.max(1),
                    "shape": "rect", "size": [0, 0], "depth": 0, "imported": true, "in_pockets": c.pockets,
                    "why": { "length": "", "width": "", "depth": "" },
                })
            })
            .collect();
        for c in &p.copies {
            let Some((lo, hi)) = copy_bounds(s, p, c) else { continue };
            let name = match (&c.name, p.copies.len() > 1) {
                (Some(n), true) => format!("{} ({n})", p.name),
                _ => p.name.clone(),
            };
            let r = |v: f64| (v * 100.0).round() / 100.0;
            out.push(json!({
                "id": tray_id(p, c), "name": name, "auto_name": p.auto_name.clone().unwrap_or_else(|| p.name.clone()),
                "request": p.request, "layer": c.layer.unwrap_or(1).saturating_sub(1),
                "pos": lo.map(r), "size": [r(hi[0] - lo[0]), r(hi[1] - lo[1]), r(hi[2] - lo[2])],
                "wall": 0, "floor": 0, "color": colour(c.filament.unwrap_or(p.filament)), "pockets": held,
                "bed_fit": p.size[0].min(p.size[1]) <= 256.0 && p.size[0].max(p.size[1]) <= 256.0 && p.size[2] <= 256.0,
                "imported": true, "piece": p.id, "design": p.design.is_some(), "copy": c.name, "inside": c.inside, "source": p.source, "piece_pockets": p.pockets,
                "why": {
                    "height": format!("as imported: {:.1} × {:.1} × {:.1} mm as it prints", p.size[0], p.size[1], p.size[2]),
                    "footprint": c.inside.as_ref().map_or_else(|| "placed in the box by hand (place_piece)".to_string(), |i| format!("inside {i}")),
                    "walls": "as designed; imported pieces aren't regenerated",
                },
            }));
        }
    }
    out
}

/// A piece's compartments, from its mesh as it prints: a height map from above on a 1 mm grid
/// (the highest surface over each cell), then the cells whose floor is more than 1.5 mm below the
/// rim, joined where they touch. Walls stand at the rim, so they part one pocket from the next;
/// holes through the floor have no surface and are left out, so a pocket can't leak out through a
/// grab notch. Pockets under 8 × 8 mm are dropped. Rects are each pocket's bounding box.
pub fn detect_pockets(games: &Path, slug: &str, p: &Piece) -> Result<Vec<crate::spec::PiecePocket>, String> {
    const CELL: f64 = 1.0;
    let (nx, ny) = ((p.size[0] / CELL).ceil() as usize + 1, (p.size[1] / CELL).ceil() as usize + 1);
    let mut top = vec![f64::NAN; nx * ny];
    for part in &p.parts {
        let tris = crate::geometry::read_stl(&dir(games, slug, &p.id).join(&part.file))?;
        for t in tris.chunks(3) {
            let v: Vec<[f64; 3]> = t.iter().map(|a| a.map(f64::from)).collect();
            // Upward-facing triangles only: they are what's seen from above.
            let n = (v[1][0] - v[0][0]) * (v[2][1] - v[0][1]) - (v[1][1] - v[0][1]) * (v[2][0] - v[0][0]);
            if n.abs() < 1e-9 {
                continue;
            }
            let (x0, x1) = (v.iter().map(|a| a[0]).fold(f64::MAX, f64::min), v.iter().map(|a| a[0]).fold(f64::MIN, f64::max));
            let (y0, y1) = (v.iter().map(|a| a[1]).fold(f64::MAX, f64::min), v.iter().map(|a| a[1]).fold(f64::MIN, f64::max));
            for i in ((x0 / CELL).floor().max(0.0) as usize)..=((x1 / CELL).ceil() as usize).min(nx - 1) {
                for j in ((y0 / CELL).floor().max(0.0) as usize)..=((y1 / CELL).ceil() as usize).min(ny - 1) {
                    let (px, py) = ((i as f64 + 0.5) * CELL, (j as f64 + 0.5) * CELL);
                    // Barycentric: is the cell centre inside, and the height there.
                    let d = (v[1][1] - v[2][1]) * (v[0][0] - v[2][0]) + (v[2][0] - v[1][0]) * (v[0][1] - v[2][1]);
                    let a = ((v[1][1] - v[2][1]) * (px - v[2][0]) + (v[2][0] - v[1][0]) * (py - v[2][1])) / d;
                    let b = ((v[2][1] - v[0][1]) * (px - v[2][0]) + (v[0][0] - v[2][0]) * (py - v[2][1])) / d;
                    let c = 1.0 - a - b;
                    if a < -1e-6 || b < -1e-6 || c < -1e-6 {
                        continue;
                    }
                    let z = a * v[0][2] + b * v[1][2] + c * v[2][2];
                    let k = j * nx + i;
                    if top[k].is_nan() || z > top[k] {
                        top[k] = z;
                    }
                }
            }
        }
    }
    let rim = p.size[2];
    let low = |k: usize| !top[k].is_nan() && top[k] < rim - 1.5;
    let mut seen = vec![false; nx * ny];
    let mut out = vec![];
    for start in 0..nx * ny {
        if seen[start] || !low(start) {
            continue;
        }
        let (mut stack, mut cells) = (vec![start], vec![]);
        seen[start] = true;
        while let Some(k) = stack.pop() {
            cells.push(k);
            let (i, j) = (k % nx, k / nx);
            let mut near = vec![];
            if i > 0 { near.push(k - 1) }
            if i + 1 < nx { near.push(k + 1) }
            if j > 0 { near.push(k - nx) }
            if j + 1 < ny { near.push(k + nx) }
            for q in near {
                if !seen[q] && low(q) {
                    seen[q] = true;
                    stack.push(q);
                }
            }
        }
        let (mut i0, mut i1, mut j0, mut j1) = (usize::MAX, 0, usize::MAX, 0);
        let mut floor = f64::MAX;
        for &k in &cells {
            let (i, j) = (k % nx, k / nx);
            (i0, i1, j0, j1) = (i0.min(i), i1.max(i), j0.min(j), j1.max(j));
            floor = floor.min(top[k]);
        }
        let (w, d) = ((i1 - i0 + 1) as f64 * CELL, (j1 - j0 + 1) as f64 * CELL);
        if w < 8.0 || d < 8.0 {
            continue;
        }
        let r = |v: f64| (v * 10.0).round() / 10.0;
        out.push(crate::spec::PiecePocket { pos: [r(i0 as f64 * CELL), r(j0 as f64 * CELL)], size: [r(w), r(d)], floor: r(floor.max(0.0)), depth: r(rim - floor.max(0.0)) });
    }
    // Reading order as the tray prints: back row first (high y), left to right.
    out.sort_by(|a, b| (b.pos[1] + b.size[1]).total_cmp(&(a.pos[1] + a.size[1])).then(a.pos[0].total_cmp(&b.pos[0])));
    Ok(out)
}

/// A component's footprint (long side first) and stack height, from how it's stored: cards,
/// tokens, minis, bins, or an as-is item's measure. None when nothing gives a size.
fn content_dims(c: &crate::spec::Component, share: usize) -> Option<[f64; 3]> {
    let k = serde_json::to_value(&c.kind).ok()?;
    let num = |v: &serde_json::Value| v.as_f64();
    let count = k["count"].as_u64().unwrap_or(1) as f64 / share.max(1) as f64;
    let (a, b, h) = match k["kind"].as_str()? {
        "cards" => (num(&k["size"][0])?, num(&k["size"][1])?, k["stack"].as_f64().unwrap_or(count * 0.31)),
        "tokens" => {
            let w = num(&k["size"][0])?;
            (w, k["size"][1].as_f64().unwrap_or(w), num(&k["thickness"])? * count)
        }
        "mini" => (num(&k["base"])?, num(&k["base"])?, num(&k["height"])?),
        "bin" => (num(&k["size"][0])?, num(&k["size"][1])?, num(&k["depth"])?),
        _ => {
            let m = c.measure.as_ref()?.piece.or(c.measure.as_ref()?.stack)?;
            (m[0], m[1], m[2])
        }
    };
    Some([a.max(b), a.min(b), h])
}

fn box_tris(lo: [f64; 3], hi: [f64; 3]) -> Vec<[f64; 3]> {
    let p = |i: usize| [if i & 1 == 0 { lo[0] } else { hi[0] }, if i & 2 == 0 { lo[1] } else { hi[1] }, if i & 4 == 0 { lo[2] } else { hi[2] }];
    // Two triangles per face, wound outwards.
    let faces = [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]];
    faces.iter().flat_map(|f| [p(f[0]), p(f[1]), p(f[2]), p(f[0]), p(f[2]), p(f[3])]).collect()
}

/// What placed copies hold, as the viewer's ghosts: each component kept in a pocket, its footprint
/// turned to the pocket's long side and kept inside it, as tall as its stack (no taller than the
/// pocket), standing on the pocket floor; written under `out`.
pub fn content_meshes(s: &Spec, box_id: &str, out: &Path) -> Result<Vec<crate::geometry::MeshIn>, String> {
    let first = s.boxes().first().map(|b| b.id.clone()).unwrap_or_default();
    let mut meshes = vec![];
    for p in s.pieces.iter().filter(|p| p.box_.as_deref().unwrap_or(&first) == box_id && !p.pockets.is_empty()) {
        for (ci, c) in p.copies.iter().enumerate() {
            let Some(m) = copy_to_box(s, p, c) else { continue };
            for comp in s.components.iter().filter(|x| x.piece.as_deref() == Some(p.id.as_str()) && !x.pockets.is_empty()) {
                for &pi in &comp.pockets {
                    let Some(q) = p.pockets.get(pi) else { continue };
                    let Some([l, w, h]) = content_dims(comp, comp.pockets.len()) else { continue };
                    let along_x = q.size[0] >= q.size[1];
                    let (gx, gy) = if along_x { (l, w) } else { (w, l) };
                    let (mut gx, mut gy) = (gx.min(q.size[0] - 1.0).max(1.0), gy.min(q.size[1] - 1.0).max(1.0));
                    let mut gh = h.min(q.depth - 0.3).max(0.5);
                    // A stack too tall for the pocket is a loose pile: it fills the floor, as deep
                    // as the pieces' volume makes it.
                    if h > q.depth - 0.3 {
                        let vol = l * w * h;
                        (gx, gy) = (q.size[0] - 1.0, q.size[1] - 1.0);
                        gh = (vol / (gx * gy)).clamp(0.5, q.depth - 0.3);
                    }
                    let cx = q.pos[0] + q.size[0] / 2.0;
                    let cy = q.pos[1] + q.size[1] / 2.0;
                    // A hair off the floor, so the two don't z-fight.
                    let z = q.floor + 0.3;
                    let tris: Vec<[f64; 3]> = box_tris([cx - gx / 2.0, cy - gy / 2.0, z], [cx + gx / 2.0, cy + gy / 2.0, z + gh]).into_iter().map(|v| plates::apply(&m, v)).collect();
                    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
                    let stl = out.join(format!("ghost-{}-{ci}-{}-{pi}.stl", p.id, comp.id));
                    write_stl(&stl, &tris)?;
                    meshes.push(crate::geometry::MeshIn {
                        name: format!("ghost-{}-{ci}-{}-{pi}", p.id, comp.id),
                        stl,
                        color: "#d8dde6".into(),
                        opacity: 0.45,
                        extras: serde_json::json!({
                            "kind": "ghost", "tray": tray_id(p, c), "layer": c.layer.unwrap_or(1).saturating_sub(1),
                            "component": comp.id, "label": comp.name,
                        }),
                    });
                }
            }
        }
    }
    Ok(meshes)
}
