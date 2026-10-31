//! BoxInsertDesigner insert server: serves the web front end, the command API, live update
//! events and the generated geometry. Binds loopback or a private address only; publish it on a
//! private network (see CLAUDE.md, "Running it as a service").

mod agent;
mod commands;
mod config;
mod edit;
mod geometry;
mod history;
mod labels;
mod layout;
mod bambu;
mod media;
mod pieces;
mod plates;
mod sample;
mod spec;
mod storage;
mod style;

use std::collections::{BTreeMap, HashMap};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::{Path as UrlPath, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{Mutex, broadcast};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;

const DEFAULT_BIND: &str = "127.0.0.1:8102";

/// Why something can't be done, in a form both Claude and the UI can act on.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Reason {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
}

impl Reason {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Reason { code: code.into(), message: message.into(), subject: None }
    }
    pub fn subject(mut self, s: &str) -> Self {
        self.subject = Some(s.into());
        self
    }
}

#[derive(Default)]
struct GameRt {
    version: u64,
    mtime: Option<SystemTime>,
    spec_error: Option<Reason>,
    building: Option<u64>,
    /// The latest finished build: one per box that has trays.
    build: Option<(u64, BTreeMap<String, geometry::Build>)>,
    build_error: Option<(u64, Reason)>,
}

pub struct App {
    games: PathBuf,
    cli: PathBuf,
    solidcraft_tag: String,
    tx: broadcast::Sender<Value>,
    rt: Mutex<HashMap<String, GameRt>>,
    /// Serialises spec writes: load → apply → validate → save.
    write: Mutex<()>,
    /// Style files as last seen, to rebuild every game when one is edited by hand.
    styles_seen: Mutex<HashMap<PathBuf, Option<SystemTime>>>,
    /// Save versioning: the games repository's history.
    history: history::History,
    /// The agents working the request queue: one outside (bid, MCP), and the app's own.
    agent: agent::Agent,
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

impl App {
    fn emit(&self, v: Value) {
        let _ = self.tx.send(v);
    }

    pub async fn current(&self) -> Option<String> {
        let games = spec::list_games(&self.games);
        let saved = std::fs::read_to_string(self.games.join(".current")).ok().map(|s| s.trim().to_string());
        match saved {
            Some(s) if games.contains(&s) => Some(s),
            _ => games.into_iter().next(),
        }
    }

    pub async fn set_current(&self, slug: &str) {
        let _ = std::fs::write(self.games.join(".current"), slug);
        self.emit(json!({ "type": "current", "game": slug }));
    }

    /// The spec on disk changed (by a command or by hand): bump the version, tell browsers, and
    /// rebuild the geometry in the background.
    pub async fn changed(self: &Arc<Self>, slug: &str) -> Result<(), Reason> {
        let path = spec::spec_path(&self.games, slug);
        let loaded = spec::load(&self.games, slug);
        let version = {
            let mut rt = self.rt.lock().await;
            let g = rt.entry(slug.to_string()).or_default();
            g.version = (g.version + 1).max(now_ms());
            g.mtime = mtime(&path);
            g.spec_error = loaded.as_ref().err().cloned();
            if loaded.is_ok() {
                g.building = Some(g.version);
            }
            g.version
        };
        // Textures first, so a browser that refetches on this event finds them.
        if let Ok(s) = &loaded {
            crate::media::ensure_faces(&self.games, slug, s);
        }
        self.emit(json!({ "type": "spec", "game": slug, "version": version }));
        let s = loaded?;
        let app = self.clone();
        let slug = slug.to_string();
        tokio::spawn(async move { app.build(&slug, version, s).await });
        Ok(())
    }

    async fn build(self: Arc<Self>, slug: &str, version: u64, s: spec::Spec) {
        let vdir = self.games.join(slug).join(".build").join(format!("v{version}"));
        let _ = std::fs::remove_dir_all(&vdir);
        self.emit(json!({ "type": "building", "game": slug, "version": version }));
        let cache = self.games.join(slug).join(".build").join("trays");
        let mut res: Result<BTreeMap<String, geometry::Build>, Reason> = Ok(BTreeMap::new());
        for gb in &s.boxes() {
            let lay = layout::chosen(&s, gb);
            let extra = match pieces::view_meshes(&s, &self.games, slug, &gb.id, &vdir.join(&gb.id).join("pieces"))
                .and_then(|mut m| pieces::content_meshes(&s, &gb.id, &vdir.join(&gb.id).join("pieces")).map(|g| { m.extend(g); m }))
            {
                Ok(m) => m,
                Err(e) => {
                    res = Err(Reason::new("pieces", format!("box {}: {e}", gb.id)));
                    break;
                }
            };
            if lay.trays.is_empty() && lay.printed.is_none() && extra.is_empty() {
                continue;
            }
            match geometry::build(&self.cli, &self.solidcraft_tag, &lay, &vdir.join(&gb.id), &cache, extra, &s.label_color()).await {
                Ok(b) => {
                    eprintln!(
                        "{slug}/{} v{version}: {} trays ({} reused), {} commands, slowest solidcraft run {} ms, total {} ms",
                        gb.id,
                        b.trays.len(),
                        b.reused,
                        b.commands,
                        b.solidcraft_ms,
                        b.total_ms
                    );
                    if let Ok(m) = &mut res {
                        m.insert(gb.id.clone(), b);
                    }
                }
                Err(e) => {
                    res = Err(Reason { message: format!("box {}: {}", gb.id, e.message), ..e });
                    break;
                }
            }
        }
        let mut rt = self.rt.lock().await;
        let g = rt.entry(slug.to_string()).or_default();
        if g.version != version {
            // A newer change superseded this build.
            let _ = std::fs::remove_dir_all(&vdir);
            return;
        }
        g.building = None;
        match res {
            Ok(b) => {
                let (ms, total) = b.values().fold((0, 0), |(a, t), x| (a.max(x.solidcraft_ms), t + x.total_ms));
                if let Some((old, _)) = g.build.replace((version, b)) {
                    if old != version {
                        let _ = std::fs::remove_dir_all(self.games.join(slug).join(".build").join(format!("v{old}")));
                    }
                }
                g.build_error = None;
                self.emit(json!({ "type": "geometry", "game": slug, "version": version, "solidcraft_ms": ms, "total_ms": total }));
            }
            Err(e) => {
                eprintln!("{slug} v{version}: build failed: {}", e.message);
                g.build_error = Some((version, e.clone()));
                self.emit(json!({ "type": "build_error", "game": slug, "version": version, "reason": e }));
            }
        }
    }

    pub async fn mutate(
        self: &Arc<Self>,
        slug: &str,
        f: impl FnOnce(&mut spec::Spec) -> Result<String, Reason>,
    ) -> Result<String, Reason> {
        let msg = {
            let _w = self.write.lock().await;
            let mut s = spec::load(&self.games, slug)?;
            let msg = f(&mut s)?;
            s.validate()?;
            spec::save(&self.games, slug, &s)?;
            msg
        };
        self.changed(slug).await?;
        Ok(msg)
    }

    /// The print plates, with a support check on every piece turned over from how it prints by
    /// default (from the trays' cached print STLs; a piece not built yet goes unchecked).
    pub fn plates_json(&self, slug: &str, s: &spec::Spec) -> Value {
        let mut pl = plates::arrange(s);
        let cache = self.games.join(slug).join(".build").join("trays");
        let mut checks = serde_json::Map::new();
        for p in pl.pieces.iter().filter(|p| p.flip) {
            let Some(gb) = s.game_box(&p.box_) else { continue };
            let lay = layout::chosen(s, &gb);
            let verdict = match geometry::cached_print_stl(&cache, &self.solidcraft_tag, &lay, &p.part) {
                None => json!({ "checked": false }),
                Some(stl) => match geometry::plate_overhang(&stl, &p.print_m, s.rules.max_overhang_deg) {
                    Ok(None) => json!({ "checked": true, "supports": false }),
                    Ok(Some(mm2)) => {
                        let msg = format!("{} upside down has {mm2:.0} mm² of overhang flatter than {}°: it would need supports", p.name, s.rules.max_overhang_deg);
                        pl.problems.push(Reason::new("needs_supports", msg).subject(&p.id));
                        json!({ "checked": true, "supports": true, "overhang_mm2": mm2.round() })
                    }
                    Err(e) => json!({ "checked": false, "error": e }),
                },
            };
            checks.insert(p.id.clone(), verdict);
        }
        let mut v = json!(pl);
        v["flip_checks"] = Value::Object(checks);
        v["presets"] = json!(bambu::presets());
        v["colours"] = Value::Object(bambu::colours().clone());
        // Each slot's colour by its Bambu name, when it is one of its preset's colours.
        if let Some(fs) = v["filaments"].as_array_mut() {
            for f in fs {
                let name = bambu::colour_name(f["preset"].as_str().unwrap_or(""), f["color"].as_str().unwrap_or(""));
                f["colour_name"] = json!(name);
            }
        }
        v
    }

    pub async fn state_json(&self, slug: &str) -> Result<Value, Reason> {
        let s = spec::load(&self.games, slug);
        let rt = self.rt.lock().await;
        let g = rt.get(slug);
        let built = g.and_then(|g| g.build.as_ref());
        let boxes: Vec<Value> = match &s {
            Ok(s) => s
                .boxes()
                .iter()
                .map(|gb| {
                    let build = built.and_then(|(v, m)| m.get(&gb.id).map(|b| (v, b))).map(|(v, b)| {
                        json!({
                            "version": v, "glb": format!("/api/games/{slug}/scene.glb?box={}&v={v}", gb.id),
                            "solidcraft_ms": b.solidcraft_ms, "total_ms": b.total_ms, "commands": b.commands, "reused": b.reused, "warnings": b.warnings,
                        })
                    });
                    let layouts = layout::generate_all(s, gb);
                    // A printed box's inside is whatever its chosen layout sized it to.
                    let shell = layouts.iter().find(|l| l.strategy == gb.layout.strategy).and_then(|l| l.printed.clone());
                    let mut size = gb.size.clone();
                    if let Some(sh) = &shell {
                        size.inner = sh.inner;
                    }
                    // Imported pieces placed in this box join the chosen layout as trays, with a layer
                    // for each guide layer the packer's layout doesn't have.
                    let mut layouts = json!(layouts);
                    let imported = pieces::trays(s, &gb.id);
                    if let Some(l) = layouts.as_array_mut().and_then(|a| a.iter_mut().find(|l| l["strategy"] == json!(gb.layout.strategy))) {
                        if !imported.is_empty() {
                            let top = imported.iter().filter_map(|t| t["layer"].as_u64()).max().unwrap_or(0) as usize;
                            let layers = l["layers"].as_array_mut().unwrap();
                            for li in layers.len()..=top {
                                let on: Vec<&Value> = imported.iter().filter(|t| t["layer"].as_u64() == Some(li as u64) && t["inside"].is_null()).collect();
                                let z = on.iter().filter_map(|t| t["pos"][2].as_f64()).fold(f64::MAX, f64::min);
                                let h = on.iter().filter_map(|t| Some(t["pos"][2].as_f64()? + t["size"][2].as_f64()?)).fold(0.0, f64::max);
                                let z = if z == f64::MAX { 0.0 } else { z };
                                layers.push(json!({ "z": z, "height": (h - z).max(0.0), "coverage": 0 }));
                            }
                            l["trays"].as_array_mut().unwrap().extend(imported);
                        }
                    }
                    json!({
                        "id": gb.id, "name": gb.name, "size": size, "chosen": gb.layout.strategy, "tweaks": gb.layout.tweaks,
                        "printed": gb.printed, "shell": shell, "layouts": layouts, "build": build,
                    })
                })
                .collect(),
            Err(_) => vec![],
        };
        Ok(json!({
            "game": slug,
            "version": g.map(|g| g.version),
            "spec": s.as_ref().ok(),
            "spec_error": s.as_ref().err().or(g.and_then(|g| g.spec_error.as_ref())),
            "boxes": boxes,
            "placement": s.as_ref().ok().map(commands::placement),
            "plates": s.as_ref().ok().map(|s| self.plates_json(slug, s)),
            "print_glb": format!("/api/games/{slug}/print.glb?v={}", built.map_or(0, |(v, _)| *v)),
            // Where each placed copy of an imported piece sits in its box: min and max corners.
            "piece_bounds": s.as_ref().ok().map(|s| {
                s.pieces.iter().map(|p| (p.id.clone(), json!(p.copies.iter().map(|c| pieces::copy_bounds(s, p, c)).collect::<Vec<_>>()))).collect::<serde_json::Map<_, _>>()
            }),
            "style": s.as_ref().ok().map(|s| json!(s.rules)),
            "textures": s.as_ref().ok().map(|s| {
                let mut t = serde_json::Map::new();
                for c in &s.components {
                    let faces: serde_json::Map<String, Value> = c.faces.iter()
                        .map(|(k, f)| (k.clone(), json!(format!("/api/games/{slug}/media/faces/{}-{k}.jpg?v={}", c.id, media::face_version(f)))))
                        .collect();
                    if !faces.is_empty() {
                        t.insert(c.id.clone(), Value::Object(faces));
                    }
                }
                Value::Object(t)
            }),
            "strategies": layout::STRATEGIES.iter().map(|(id, t, d)| json!({ "id": id, "title": t, "description": d })).collect::<Vec<_>>(),
            "storage_kinds": storage::KINDS,
            "history": self.history.status(slug).await,
            "building": g.and_then(|g| g.building).is_some(),
            "build_error": g.and_then(|g| g.build_error.as_ref()).map(|(v, e)| json!({ "version": v, "reason": e })),
            "solidcraft": self.solidcraft_tag,
        }))
    }

    /// Print files for every box's chosen layout into `out/`. With one box the names are
    /// `<slug>-<tray>.stl`; with several, `<slug>-<box>-<tray>.stl`.
    pub async fn export(self: &Arc<Self>, slug: &str) -> Result<Value, Reason> {
        let s = spec::load(&self.games, slug)?;
        let boxes = s.boxes();
        let lays: Vec<_> = boxes.iter().map(|gb| (gb, layout::chosen(&s, gb))).filter(|(_, l)| !l.trays.is_empty()).collect();
        if lays.is_empty() && s.pieces.is_empty() {
            return Err(Reason::new("nothing_to_export", "no box has trays in its chosen layout, and there are no imported pieces"));
        }
        let many = boxes.len() > 1;
        let stem = |gb: &spec::GameBox| if many { format!("{slug}-{}", gb.id) } else { slug.to_string() };
        let root = self.games.join(slug).join(".build").join(format!("export-{}", now_ms()));
        let mut built = vec![];
        let mut ms = 0;
        let mut warnings = vec![];
        for (gb, lay) in &lays {
            let dir = root.join(&gb.id);
            let (m, w) = geometry::build_full(&self.cli, lay, &dir).await?;
            ms += m;
            warnings.extend(w);
            // A tray kept as a SolidCraft edit prints as that (the box-wide insert 3MF, made in
            // the one run, still shows it as generated).
            for t in lay.trays.iter().filter(|t| t.kept_edit.is_some()) {
                let kdir = dir.join(format!("kept-{}", t.id));
                if edit::build_kept(&self.cli, &self.games.join(slug), lay, t, &self.solidcraft_tag, &kdir, &mut warnings).await.is_ok() {
                    std::fs::copy(kdir.join("print").join(format!("{}.stl", t.id)), dir.join("print").join(format!("{}.stl", t.id))).map_err(|e| Reason::new("io", e.to_string()))?;
                }
            }
            built.push((*gb, lay, dir));
        }
        let out = self.games.join(slug).join("out");
        std::fs::create_dir_all(&out).map_err(|e| Reason::new("io", e.to_string()))?;
        for e in std::fs::read_dir(&out).into_iter().flatten().flatten() {
            let p = e.path();
            if matches!(p.extension().and_then(|x| x.to_str()), Some("stl" | "3mf" | "json")) {
                let _ = std::fs::remove_file(p);
            }
        }
        let io = |e: std::io::Error| Reason::new("io", e.to_string());
        let mut files = vec![];
        let mut printability = vec![];
        let mut problems = vec![];
        let mut unconfirmed = vec![];
        for (gb, lay, dir) in &built {
            for t in &lay.trays {
                if let Err(e) = geometry::check_tray_stl(&dir.join("print").join(format!("{}.stl", t.id)), t, s.rules.max_overhang_deg) {
                    printability.push(Reason::new("not_printable", e).subject(&t.id));
                }
                let name = format!("{}-{}.stl", stem(gb), t.id);
                std::fs::copy(dir.join("print").join(format!("{}.stl", t.id)), out.join(&name)).map_err(io)?;
                let (lo, hi) = t.print_bounds();
                let size: Vec<f64> = (0..3).map(|i| ((hi[i] - lo[i]) * 1000.0).round() / 1000.0).collect();
                files.push(json!({ "file": name, "box": gb.id, "tray": t.id, "name": t.name, "layer": t.layer + 1, "size_mm": size, "bed_fit": t.bed_fit }));
                // An inlaid label: its own STL too, for a slicer that colours by object.
                if let Some(src) = geometry::label_stl(dir, "print", t) {
                    if let Err(e) = geometry::check_label_stl(&src, t) {
                        printability.push(Reason::new("not_printable", e).subject(&t.id));
                    }
                    let name = format!("{}-{}-label.stl", stem(gb), t.id);
                    std::fs::copy(&src, out.join(&name)).map_err(io)?;
                    let slot = s.label_slot();
                    files.push(json!({ "file": name, "box": gb.id, "tray": t.id, "name": format!("{}: label", t.name), "filament": slot, "note": format!("fills the text cut into {}, flush; filament slot {slot}", t.id) }));
                }
            }
            if let Some(sh) = &lay.printed {
                for (part, size) in geometry::shell_print_sizes(sh) {
                    let src = dir.join("print").join(format!("{part}.stl"));
                    if let Err(e) = geometry::check_stl(&src, size, s.rules.max_overhang_deg) {
                        printability.push(Reason::new("not_printable", e).subject(&gb.id));
                    }
                    let name = format!("{}-{part}.stl", stem(gb));
                    std::fs::copy(&src, out.join(&name)).map_err(io)?;
                    let what = if part == geometry::PRINTED_BOX { "the printed box, upright" } else { "its slide-on lid, top face down" };
                    files.push(json!({ "file": name, "box": gb.id, "name": what, "size_mm": size, "bed_fit": sh.bed_fit }));
                }
            }
            let mf = format!("{}-insert.3mf", stem(gb));
            std::fs::copy(dir.join("insert.3mf"), out.join(&mf)).map_err(io)?;
            files.push(json!({ "file": mf, "box": gb.id, "note": "all trays as named objects, in their places in the box; arrange in the slicer" }));
            problems.extend(lay.problems.iter().cloned().map(|p| Reason { message: format!("{}: {}", gb.id, p.message), ..p }));
            unconfirmed.extend(lay.report.unconfirmed.iter().cloned());
        }
        // The Bambu Studio project: every box's pieces on the X1C's plates, as the Plate tab shows.
        let pl = plates::arrange(&s);
        for p in pl.pieces.iter().filter(|p| p.flip) {
            let Some((_, _, dir)) = built.iter().find(|(gb, _, _)| gb.id == p.box_) else { continue };
            match geometry::plate_overhang(&dir.join("print").join(format!("{}.stl", p.part)), &p.print_m, s.rules.max_overhang_deg) {
                Ok(None) => {}
                Ok(Some(mm2)) => printability.push(Reason::new("not_printable", format!("{} upside down: {mm2:.0} mm² of overhang would need supports", p.name)).subject(&p.id)),
                Err(e) => printability.push(Reason::new("not_printable", e).subject(&p.id)),
            }
        }
        problems.extend(pl.problems.iter().cloned());
        let project = format!("{slug}-plates.3mf");
        let parts_of = |p: &plates::Piece| -> bambu::PartFiles {
            if let Some(ip) = s.pieces.iter().find(|x| format!("piece/{}", x.id) == p.id) {
                let d = pieces::dir(&self.games, slug, &ip.id);
                return ip.parts.iter().zip(&p.parts).map(|(q, slot)| (q.name.clone(), slot.filament, d.join(&q.file))).collect();
            }
            let (dir, lay) = built.iter().find(|(gb, _, _)| gb.id == p.box_).map(|(_, l, d)| (d.clone(), Some(*l))).unwrap_or_default();
            let label = lay.and_then(|l| l.trays.iter().find(|t| t.id == p.part)).and_then(|t| geometry::label_stl(&dir, "print", t));
            // The parts as the plate lists them: the tray, then its label.
            std::iter::once(dir.join("print").join(format!("{}.stl", p.part)))
                .chain(label)
                .zip(&p.parts)
                .map(|(stl, slot)| (slot.name.clone(), slot.filament, stl))
                .collect()
        };
        bambu::write_project(&out.join(&project), &s.game.name, &pl, &s.filament_slots(), &parts_of).map_err(|e| Reason::new("bambu_3mf", e))?;
        files.push(json!({ "file": project, "plates": pl.count, "note": format!("Bambu Studio project for the {}: {} pieces on {} plates, as the Plate tab shows them; open it and slice", s.printer.name, pl.pieces.len(), pl.count) }));
        unconfirmed.sort();
        unconfirmed.dedup();
        let strategies: BTreeMap<_, _> = built.iter().map(|(gb, l, _)| (gb.id.clone(), l.strategy.clone())).collect();
        let manifest = json!({
            "game": slug, "strategies": strategies, "exported_ms": now_ms(), "solidcraft": self.solidcraft_tag,
            "printer": s.printer.name, "ok": problems.is_empty() && printability.is_empty(), "problems": problems, "printability": printability, "files": files,
            "unconfirmed": unconfirmed,
        });
        std::fs::write(out.join("manifest.json"), serde_json::to_vec_pretty(&manifest).unwrap()).map_err(io)?;
        let _ = std::fs::remove_dir_all(&root);
        let shown: Vec<Value> = files.iter().map(|f| f["file"].clone()).collect();
        self.emit(json!({ "type": "export", "game": slug, "files": shown, "dir": out.display().to_string() }));
        Ok(json!({ "dir": out.display().to_string(), "files": files, "problems": problems, "printability": printability, "warnings": warnings, "solidcraft_ms": ms }))
    }

    /// Pick up hand edits to spec and style files.
    async fn watch(self: Arc<Self>) {
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let styles: Vec<PathBuf> = std::fs::read_dir(style::dir()).into_iter().flatten().flatten().map(|e| e.path()).collect();
            let mut edited = false;
            {
                let mut seen = self.styles_seen.lock().await;
                for p in styles {
                    let m = mtime(&p);
                    if seen.insert(p.clone(), m).is_some_and(|old| old != m) {
                        eprintln!("style {} changed on disk", p.display());
                        edited = true;
                    }
                }
            }
            if edited {
                for slug in spec::list_games(&self.games) {
                    let _ = self.changed(&slug).await;
                }
            }
            for slug in spec::list_games(&self.games) {
                let m = mtime(&spec::spec_path(&self.games, &slug));
                let known = self.rt.lock().await.get(&slug).map(|g| g.mtime);
                if known != Some(m) && self.write.try_lock().is_ok() {
                    eprintln!("{slug}: spec changed on disk");
                    self.history.note(&slug, "spec edited by hand").await;
                    let _ = self.changed(&slug).await;
                }
            }
        }
    }
}

async fn health() -> Json<Value> {
    Json(json!({ "ok": true, "service": "box-insert-designer", "version": env!("CARGO_PKG_VERSION") }))
}

/// Commands that don't change a game, so leave no line in its history.
const READ_ONLY: [&str; 20] = [
    "list_games", "use_game", "get_state", "generate_layouts", "requests", "wait_requests", "export", "set_style",
    "history", "save_version", "restore_version", "help", "tray_history", "agent_status", "set_agent", "",
    // submit_edit notes its own line: the history shouldn't hold the design's bytes.
    "edit_script", "edit_diff", "submit_edit", "piece_design",
];

/// One command, as `/api/command` takes it: the web app, `bid`, and the app's own agent.
pub async fn run_command(app: &Arc<App>, body: Value) -> Result<Value, Reason> {
    let cmd = body["cmd"].as_str().unwrap_or("").to_string();
    let req: commands::Request = serde_json::from_value(body).map_err(|e| Reason::new("bad_command", e.to_string()))?;
    let v = commands::run(app, req).await?;
    if let (false, Some(game)) = (READ_ONLY.contains(&cmd.as_str()), v["game"].as_str()) {
        app.history.note(game, &format!("{cmd}: {}", v["message"].as_str().unwrap_or(""))).await;
    }
    Ok(v)
}

async fn command(State(app): State<Arc<App>>, headers: axum::http::HeaderMap, Json(body): Json<Value>) -> Response {
    // An agent working the queue from outside (bid, MCP) shows as watching while it polls.
    let cmd = body["cmd"].as_str().unwrap_or("").to_string();
    let _watch = agent::Outside::seen(&app.agent, &cmd, headers.get("x-bid-client").and_then(|v| v.to_str().ok()));
    match run_command(&app, body).await {
        Ok(v) => Json(v).into_response(),
        Err(r) => (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "ok": false, "reason": r }))).into_response(),
    }
}

async fn state(State(app): State<Arc<App>>, UrlPath(slug): UrlPath<String>) -> Response {
    match app.state_json(&slug).await {
        Ok(v) => Json(v).into_response(),
        Err(r) => (StatusCode::NOT_FOUND, Json(json!({ "ok": false, "reason": r }))).into_response(),
    }
}

async fn games(State(app): State<Arc<App>>) -> Json<Value> {
    let games = spec::list_games(&app.games);
    // Their names, for the app's project switcher.
    let names: BTreeMap<_, _> = games.iter().map(|g| (g.clone(), spec::load(&app.games, g).map(|s| s.game.name).unwrap_or_else(|_| g.clone()))).collect();
    Json(json!({ "games": games, "names": names, "current": app.current().await }))
}

async fn glb(State(app): State<Arc<App>>, UrlPath(slug): UrlPath<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let rt = app.rt.lock().await;
    let builds = rt.get(&slug).and_then(|g| g.build.as_ref()).map(|(_, m)| m);
    let path = builds.and_then(|m| match q.get("box") {
        Some(b) => m.get(b),
        None => m.values().next(),
    })
    .map(|b| b.glb.clone());
    drop(rt);
    match path.and_then(|p| std::fs::read(p).ok()) {
        Some(bytes) => ([(header::CONTENT_TYPE, "model/gltf-binary")], bytes).into_response(),
        None => (StatusCode::NOT_FOUND, "no geometry yet").into_response(),
    }
}

/// Every piece as it prints, part by part (`extras`: piece, part index, filament), for the Plates
/// tab, which moves each onto its plate. Trays come from the view build's cache, so a tray not
/// built yet is missing until it is.
async fn print_glb(State(app): State<Arc<App>>, UrlPath(slug): UrlPath<String>) -> Response {
    let Ok(s) = spec::load(&app.games, &slug) else { return (StatusCode::NOT_FOUND, "no such game").into_response() };
    // The last finished build: its trays are all in the cache.
    let version = app.rt.lock().await.get(&slug).and_then(|g| g.build.as_ref().map(|(v, _)| *v)).unwrap_or(0);
    let out = app.games.join(&slug).join(".build").join(format!("print-v{version}.glb"));
    if !out.is_file() {
        let pl = plates::arrange(&s);
        let cache = app.games.join(&slug).join(".build").join("trays");
        let mut meshes = vec![];
        for p in &pl.pieces {
            let files: Vec<PathBuf> = match s.pieces.iter().find(|x| format!("piece/{}", x.id) == p.id) {
                Some(ip) => ip.parts.iter().map(|q| pieces::dir(&app.games, &slug, &ip.id).join(&q.file)).collect(),
                None => {
                    let Some(gb) = s.game_box(&p.box_) else { continue };
                    geometry::cached_print_parts(&cache, &app.solidcraft_tag, &layout::chosen(&s, &gb), &p.part)
                }
            };
            for (k, stl) in files.into_iter().enumerate() {
                meshes.push(geometry::MeshIn {
                    name: format!("{}#{k}", p.id),
                    stl,
                    color: p.color.clone(),
                    opacity: 1.0,
                    extras: json!({ "piece": p.id, "part": k, "filament": p.parts.get(k).map_or(p.filament, |x| x.filament) }),
                });
            }
        }
        // Old ones go: only the current version is asked for.
        for e in std::fs::read_dir(app.games.join(&slug).join(".build")).into_iter().flatten().flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.starts_with("print-v") && n.ends_with(".glb") {
                let _ = std::fs::remove_file(e.path());
            }
        }
        if let Err(e) = geometry::write_glb(&meshes, &out) {
            return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
        }
    }
    match std::fs::read(&out) {
        Ok(bytes) => ([(header::CONTENT_TYPE, "model/gltf-binary")], bytes).into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "no geometry yet").into_response(),
    }
}

/// Photos and face textures: `/api/games/<slug>/media/photos/<file>` or `.../faces/<file>`.
async fn media_file(State(app): State<Arc<App>>, UrlPath((slug, kind, file)): UrlPath<(String, String, String)>) -> Response {
    let safe = |s: &str| !s.is_empty() && !s.starts_with('.') && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    if !spec::is_slug(&slug) || !matches!(kind.as_str(), "photos" | "faces" | "edits") || !safe(&file) {
        return (StatusCode::NOT_FOUND, "no such file").into_response();
    }
    // An edit's pictures keep their names as they change, so they aren't cached.
    let (ctype, cache) = match kind.as_str() {
        "edits" if file.ends_with(".png") => ("image/png", "no-cache"),
        "edits" => return (StatusCode::NOT_FOUND, "no such file").into_response(),
        _ => ("image/jpeg", "max-age=86400"),
    };
    let dir = if kind == "edits" { edit::pictures_dir(&app.games, &slug) } else { app.games.join(&slug).join(&kind) };
    match std::fs::read(dir.join(&file)) {
        Ok(bytes) => ([(header::CONTENT_TYPE, ctype), (header::CACHE_CONTROL, cache)], bytes).into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "no such file").into_response(),
    }
}

/// An exported file (`games/<slug>/out/`), as a download.
async fn out_file(State(app): State<Arc<App>>, UrlPath((slug, file)): UrlPath<(String, String)>) -> Response {
    let safe = !file.starts_with('.') && file.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    let ctype = match file.rsplit('.').next() {
        Some("3mf") => "model/3mf",
        Some("stl") => "model/stl",
        Some("json") => "application/json",
        _ => "",
    };
    if !spec::is_slug(&slug) || !safe || ctype.is_empty() {
        return (StatusCode::NOT_FOUND, "no such file").into_response();
    }
    match std::fs::read(app.games.join(&slug).join("out").join(&file)) {
        Ok(bytes) => ([(header::CONTENT_TYPE, ctype.to_string()), (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{file}\""))], bytes).into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "no such file").into_response(),
    }
}

/// Every exported file of a game in one zip, `<slug>-export.zip`.
async fn out_zip(State(app): State<Arc<App>>, UrlPath(slug): UrlPath<String>) -> Response {
    if !spec::is_slug(&slug) {
        return (StatusCode::NOT_FOUND, "no such game").into_response();
    }
    let out = app.games.join(&slug).join("out");
    let zipped = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, String> {
        use std::io::Write;
        let mut names: Vec<_> = std::fs::read_dir(&out).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).filter(|p| matches!(p.extension().and_then(|x| x.to_str()), Some("stl" | "3mf"))).collect();
        names.sort();
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for p in names {
            z.start_file(p.file_name().unwrap().to_string_lossy(), opts).map_err(|e| e.to_string())?;
            z.write_all(&std::fs::read(&p).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        }
        Ok(z.finish().map_err(|e| e.to_string())?.into_inner())
    })
    .await;
    match zipped {
        Ok(Ok(bytes)) => ([(header::CONTENT_TYPE, "application/zip".to_string()), (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{slug}-export.zip\""))], bytes).into_response(),
        _ => (StatusCode::NOT_FOUND, "nothing exported yet").into_response(),
    }
}

async fn events(State(app): State<Arc<App>>) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let hello = tokio_stream::once(json!({ "type": "hello" }));
    let rx = BroadcastStream::new(app.tx.subscribe()).filter_map(|m| m.ok());
    Sse::new(hello.chain(rx).map(|v| Ok(Event::default().data(v.to_string())))).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

/// SolidCraft's web app at the pinned tag (`tools/fetch-solidcraft.sh` unzips it), if fetched.
fn solidcraft_web(tag: &str) -> Option<PathBuf> {
    let dir = Path::new("upstream").join(tag).join("web");
    let app = std::fs::read_dir(&dir).ok()?.flatten().map(|e| e.path()).find(|p| p.join("index.html").is_file())?;
    std::fs::canonicalize(app).ok()
}

fn solidcraft_cli() -> Result<(PathBuf, String), Box<dyn std::error::Error>> {
    let pin: Value = toml::from_str::<toml::Value>(&std::fs::read_to_string("upstream.toml")?)?.try_into()?;
    let tag = pin["solidcraft"]["tag"].as_str().ok_or("upstream.toml: no tag")?.to_string();
    if let Some(p) = config::server("BID_SOLIDCRAFT", "solidcraft") {
        return Ok((p.into(), tag));
    }
    // The macOS CLI ships on its own; on Windows it comes inside the portable desktop package.
    let key = if cfg!(windows) { "windows_binary" } else { "binary" };
    let bin = pin["solidcraft"][key].as_str().filter(|b| !b.is_empty()).ok_or_else(|| format!("upstream.toml: no {key} for this platform"))?;
    let p = Path::new("upstream").join(&tag).join("cli").join(bin);
    if !p.is_file() {
        return Err(format!("{} missing: run tools/fetch-solidcraft.sh (Windows: tools/fetch-solidcraft.ps1)", p.display()).into());
    }
    Ok((std::fs::canonicalize(p)?, tag))
}

/// `bid-server --smoke [GAMES_DIR]`: build every game's chosen layout with the configured
/// solidcraft-cli (live path and export path) and check each tray's STL. Prints one JSON line
/// per game; exits 1 on any failure. The daily upkeep runs this against a new release.
async fn smoke(games: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let (cli, tag) = solidcraft_cli()?;
    let tmp = std::env::temp_dir().join(format!("bid-smoke-{}", now_ms()));
    let mut failed = false;
    for slug in spec::list_games(&games) {
        let t0 = std::time::Instant::now();
        let res: Result<Value, String> = async {
            let s = spec::load(&games, &slug).map_err(|r| r.message)?;
            let mut boxes = vec![];
            for gb in &s.boxes() {
                let lay = layout::chosen(&s, gb);
                if lay.trays.is_empty() {
                    boxes.push(json!({ "box": gb.id, "trays": 0 }));
                    continue;
                }
                let dir = tmp.join(&slug).join(&gb.id);
                let live = geometry::build(&cli, &tag, &lay, &dir.join("live"), &dir.join("cache"), vec![], &s.label_color()).await.map_err(|r| r.message)?;
                let (ms, warnings) = geometry::build_full(&cli, &lay, &dir.join("full")).await.map_err(|r| r.message)?;
                let mut tris = 0;
                for t in &lay.trays {
                    tris += geometry::check_tray_stl(&dir.join("full").join("print").join(format!("{}.stl", t.id)), t, s.rules.max_overhang_deg)?;
                    if let Some(stl) = geometry::label_stl(&dir.join("full"), "print", t) {
                        tris += geometry::check_label_stl(&stl, t)?;
                    }
                }
                for (part, size) in lay.printed.iter().flat_map(geometry::shell_print_sizes) {
                    tris += geometry::check_stl(&dir.join("full").join("print").join(format!("{part}.stl")), size, s.rules.max_overhang_deg)?;
                }
                if !dir.join("full").join("insert.3mf").is_file() {
                    return Err(format!("{}: no insert.3mf", gb.id));
                }
                let all_warnings: Vec<_> = live.warnings.iter().chain(&warnings).collect();
                boxes.push(json!({ "box": gb.id, "strategy": lay.strategy, "trays": lay.trays.len(), "triangles": tris,
                                   "live_ms": live.total_ms, "full_ms": ms, "warnings": all_warnings }));
            }
            Ok(json!({ "boxes": boxes }))
        }
        .await;
        let ms = t0.elapsed().as_millis();
        match res {
            Ok(v) => println!("{}", json!({ "game": slug, "ok": true, "solidcraft": tag, "ms": ms, "result": v })),
            Err(e) => {
                failed = true;
                println!("{}", json!({ "game": slug, "ok": false, "solidcraft": tag, "ms": ms, "error": e }));
            }
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);
    if failed {
        std::process::exit(1);
    }
    Ok(())
}

/// The host names a request may be addressed to: loopback, the bound address, and
/// `[server] hosts` in bid.toml (`BID_HOSTS`), e.g. the name a private network publishes it under.
fn allowed_hosts(bind: SocketAddr) -> Vec<String> {
    let mut v: Vec<String> = ["localhost", "127.0.0.1", "[::1]"].map(String::from).into();
    v.push(match bind.ip() {
        std::net::IpAddr::V6(ip) => format!("[{ip}]"),
        ip => ip.to_string(),
    });
    v.extend(config::server_list("BID_HOSTS", "hosts").into_iter().map(|h| h.to_ascii_lowercase()));
    v
}

/// Refuses a request whose Host isn't one of ours. The server only listens on loopback or a
/// private address, but a web page could still reach it by pointing a name it controls at that
/// address (DNS rebinding); its requests then carry that name.
async fn check_host(hosts: Arc<Vec<String>>, req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let host = req.headers().get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("").to_ascii_lowercase();
    // Without the port: after the last ':' unless that's inside an IPv6 address's brackets.
    let name = match host.rfind(':') {
        Some(i) if !host[i..].contains(']') => &host[..i],
        _ => host.as_str(),
    };
    if hosts.iter().any(|h| h == name) {
        return next.run(req).await;
    }
    (StatusCode::MISDIRECTED_REQUEST, format!("host {host:?} isn't one this server answers to; add it to [server] hosts in bid.toml\n")).into_response()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--smoke") {
        let games = PathBuf::from(args.get(2).cloned().unwrap_or_else(|| "games".into()));
        return smoke(std::fs::canonicalize(games)?).await;
    }
    let bind: SocketAddr = config::server("BID_BIND", "bind").unwrap_or_else(|| DEFAULT_BIND.into()).parse()?;
    if bind.ip().is_unspecified() {
        return Err("refusing to bind a wildcard address; use loopback or the Tailscale IP".into());
    }
    let web_dir = config::server("BID_WEB_DIR", "web_dir").unwrap_or_else(|| "web".into());
    let games_dir = config::games_dir();
    let (cli, tag) = solidcraft_cli()?;
    let sc_web = solidcraft_web(&tag);
    if sc_web.is_none() {
        eprintln!("no SolidCraft web app for {tag} (tools/fetch-solidcraft.sh): Edit in SolidCraft is off");
    }
    std::fs::create_dir_all(&games_dir)?;
    let games_dir = std::fs::canonicalize(games_dir)?;
    for slug in spec::list_games(&games_dir) {
        let _ = std::fs::remove_dir_all(games_dir.join(&slug).join(".build"));
    }
    if spec::list_games(&games_dir).is_empty() {
        spec::save(&games_dir, sample::SLUG, &sample::spec()).map_err(|r| r.message)?;
    }

    let (tx, _) = broadcast::channel(256);
    let history = history::History::new(&games_dir);
    if !history.enabled() {
        eprintln!("history: {} isn't a git repository; save versioning is off", games_dir.display());
    }
    let app = Arc::new(App { games: games_dir.clone(), cli, solidcraft_tag: tag, tx, rt: Mutex::default(), write: Mutex::default(), styles_seen: Mutex::default(), history, agent: agent::Agent::load() });
    app.history.note_dirty().await;
    for slug in spec::list_games(&games_dir) {
        if let Err(e) = app.changed(&slug).await {
            eprintln!("{slug}: {}", e.message);
        }
    }
    tokio::spawn(app.clone().watch());
    tokio::spawn(agent::work(app.clone()));
    // Commit games whose changes have settled.
    let committer = app.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            for slug in committer.history.flush_due().await {
                committer.emit(json!({ "type": "history", "game": slug }));
            }
        }
    });

    let api = Router::new()
        .route("/api/command", post(command).layer(axum::extract::DefaultBodyLimit::max(32 * 1024 * 1024)))
        .route("/api/games", get(games))
        .route("/api/games/{slug}/state", get(state))
        .route("/api/games/{slug}/scene.glb", get(glb))
        .route("/api/games/{slug}/print.glb", get(print_glb))
        .route("/api/games/{slug}/out/{file}", get(out_file))
        .route("/api/games/{slug}/out.zip", get(out_zip))
        .route("/api/events", get(events))
        .layer(SetResponseHeaderLayer::overriding(header::CACHE_CONTROL, HeaderValue::from_static("no-store")));
    let app_router = Router::new()
        .route("/health", get(health))
        // Outside `api`: photos have unique names and textures a version, so they may be cached.
        .route("/api/games/{slug}/media/{kind}/{file}", get(media_file))
        .merge(api);
    // Edit in SolidCraft: the web app on this origin, so the page that opens it can drive it.
    let app_router = match &sc_web {
        // Gzipped: its wasm is 94 MB as it comes.
        Some(dir) => app_router.merge(Router::new().nest_service("/solidcraft", ServeDir::new(dir)).layer(tower_http::compression::CompressionLayer::new())),
        None => app_router,
    };
    let hosts = Arc::new(allowed_hosts(bind));
    let app_router = app_router
        .fallback_service(ServeDir::new(&web_dir))
        .layer(axum::middleware::from_fn(move |req, next| check_host(hosts.clone(), req, next)))
        .layer(SetResponseHeaderLayer::if_not_present(header::CACHE_CONTROL, HeaderValue::from_static("no-cache")))
        .with_state(app);

    let listener = tokio::net::TcpListener::bind(bind).await?;
    eprintln!("box-insert-designer listening on http://{bind} (web: {web_dir}, games: {})", games_dir.display());
    axum::serve(listener, app_router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
