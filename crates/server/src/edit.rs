//! Edit in SolidCraft: a tray goes to SolidCraft's web app (served at /solidcraft/) with its
//! full history, and what the user does there comes back to be seen, not taken.
//!
//! The spec stays the master copy. The page that opened the editor saves the edited design
//! (`file.save_as`, a `.sc`) and sends it here. It is kept as `games/<slug>/edits/<tray>.sc`
//! and compared with the tray as generated, feature by feature (`solidcraft-cli script`, which
//! gives a design's timeline back by feature id), parameter by parameter and body by body
//! (`solidcraft-cli eval`). The comparison is what the app shows and what becomes a note for
//! the agent.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::Reason;
use crate::layout::{Layout, Tray};

/// The tray's script for the editor: the tray alone, in print pose, with no mesh saves. With
/// and without scoop fillets, as the build retries without them when the kernel refuses one.
pub fn script(layout: &Layout, t: &Tray) -> (Vec<Value>, Vec<Value>) {
    (crate::geometry::edit_script(layout, t, true), crate::geometry::edit_script(layout, t, false))
}

pub fn edits_dir(games: &Path, slug: &str) -> PathBuf {
    games.join(slug).join("edits")
}

/// The pictures of the last edit of each tray (`<tray>.png`, `<tray>.generated.png`): made
/// again on the next save, so they stay out of the game's history (`.build` is ignored there).
pub fn pictures_dir(games: &Path, slug: &str) -> PathBuf {
    games.join(slug).join(".build").join("edit-pictures")
}

pub(crate) async fn cli(cli: &Path, dir: &Path, args: &[&str]) -> Result<(), Reason> {
    let out = tokio::process::Command::new(cli)
        .args(args)
        .current_dir(dir)
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| Reason::new("solidcraft_failed", format!("can't run {}: {e}", cli.display())))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let last = err.lines().filter(|l| !l.trim().is_empty()).last().unwrap_or("solidcraft-cli failed");
        return Err(Reason::new("solidcraft_failed", format!("{} {}: {last}", args.first().unwrap_or(&""), args.get(1).unwrap_or(&""))));
    }
    Ok(())
}

pub(crate) fn read_json(p: &Path) -> Result<Value, Reason> {
    let s = std::fs::read_to_string(p).map_err(|e| Reason::new("io", format!("{}: {e}", p.display())))?;
    serde_json::from_str(&s).map_err(|e| Reason::new("io", format!("{}: {e}", p.display())))
}

/// The tray as generated, as a design: `<cache>/base.sc`, built once per tray geometry.
async fn base(clip: &Path, layout: &Layout, t: &Tray, cache: &Path) -> Result<PathBuf, Reason> {
    let sc = cache.join("base.sc");
    if sc.is_file() {
        return Ok(sc);
    }
    std::fs::create_dir_all(cache).map_err(|e| Reason::new("io", e.to_string()))?;
    let (with, without) = script(layout, t);
    let mut last = None;
    for cmds in [with, without] {
        std::fs::write(cache.join("script.json"), serde_json::to_vec(&json!({ "commands": cmds })).unwrap()).map_err(|e| Reason::new("io", e.to_string()))?;
        match cli(clip, cache, &["run", "script.json", "--quiet", "--save", "base.sc"]).await {
            Ok(()) => return Ok(sc),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap())
}

/// A design's timeline (`id` → feature) and its measurement (`eval`), each one CLI run that
/// opens and rebuilds the design, so they run side by side. Files already in `dir` are reused.
async fn read_design(clip: &Path, dir: &Path, sc: &Path, stem: &str) -> Result<(BTreeMap<u64, Value>, Value), Reason> {
    let sc = sc.to_string_lossy().to_string();
    let (f, m) = (format!("{stem}.features.json"), format!("{stem}.eval.json"));
    let run = |args: Vec<String>, out: &str| {
        let fresh = !dir.join(out).is_file();
        async move {
            if fresh {
                let args: Vec<&str> = args.iter().map(String::as_str).collect();
                cli(clip, dir, &args).await?;
            }
            Ok::<_, Reason>(())
        }
    };
    let (a, b) = tokio::join!(
        run(vec!["script".into(), sc.clone(), "--out".into(), f.clone()], &f),
        run(vec!["eval".into(), sc.clone(), "--json-out".into(), m.clone()], &m)
    );
    a?;
    b?;
    let f = read_json(&dir.join(&f))?;
    let steps = f.get("commands").unwrap_or(&f).as_array().cloned().unwrap_or_default();
    let feats = steps
        .into_iter()
        .filter_map(|c| c["params"]["feature"].as_object().cloned())
        .filter_map(|f| Some((f.get("id")?.as_u64()?, Value::Object(f))))
        .collect();
    Ok((feats, read_json(&dir.join(&m))?))
}

/// An iso picture of a design, unless `png` is there already.
async fn picture(clip: &Path, dir: &Path, sc: &Path, png: &Path) -> Result<(), Reason> {
    if png.is_file() {
        return Ok(());
    }
    let (sc, png) = (sc.to_string_lossy().to_string(), png.to_string_lossy().to_string());
    cli(clip, dir, &["snapshot", "--in", &sc, "--out", &png, "--view", "iso", "--width", "800", "--height", "560", "--no-axes"]).await
}

/// Fields that say nothing to a reader: the kernel's own references and parameter names.
const QUIET: [&str; 4] = ["id", "param_names", "edge_refs", "participants"];

fn short(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => {
            let f = n.as_f64().unwrap_or(0.0);
            format!("{}", (f * 1000.0).round() / 1000.0)
        }
        Value::Array(a) if a.iter().all(Value::is_number) => format!("[{}]", a.iter().map(short).collect::<Vec<_>>().join(", ")),
        _ => {
            let s = v.to_string();
            if s.chars().count() > 80 { format!("{}…", s.chars().take(80).collect::<String>()) } else { s }
        }
    }
}

/// A feature in a few words: its name, kind, operation, and what it acts on.
fn describe(f: &Value) -> String {
    let mut s = format!("{} ({}", f["name"].as_str().unwrap_or("?"), f["type"].as_str().unwrap_or("feature"));
    if let Some(op) = f["operation"].as_str() {
        s += &format!(", {op}");
    }
    s.push(')');
    for k in ["length", "width", "height", "radius", "distance", "diameter"] {
        if let Some(v) = f.get(k) {
            s += &format!(" {k} {}", short(v));
        }
    }
    if let Some(d) = f["extent"].get("distance") {
        s += &format!(" depth {}", short(d));
    }
    if let Some(n) = f["edge_names"].as_array() {
        s += &format!(" on {} edges", n.len());
    }
    s
}

fn field_changes(a: &Value, b: &Value) -> Vec<Value> {
    let (Some(a), Some(b)) = (a.as_object(), b.as_object()) else { return vec![] };
    let mut keys: Vec<&String> = a.keys().chain(b.keys()).filter(|k| !QUIET.contains(&k.as_str())).collect();
    keys.sort();
    keys.dedup();
    keys.into_iter()
        .filter(|k| a.get(*k) != b.get(*k))
        .map(|k| {
            let show = |v: Option<&Value>| match (k.as_str(), v) {
                (_, None) => "—".to_string(),
                ("edge_names", Some(Value::Array(e))) => format!("{} edges", e.len()),
                (_, Some(v)) => short(v),
            };
            json!({ "field": k, "from": show(a.get(k)), "to": show(b.get(k)) })
        })
        .collect()
}

fn by_name(m: &Value, list: &str) -> BTreeMap<String, Value> {
    m[list].as_array().into_iter().flatten().filter_map(|v| Some((v["name"].as_str()?.to_string(), v.clone()))).collect()
}

fn size(b: &Value) -> Option<[f64; 3]> {
    let (lo, hi) = (b["bbox"]["min"].as_array()?, b["bbox"]["max"].as_array()?);
    let d = |i: usize| Some(((hi.get(i)?.as_f64()? - lo.get(i)?.as_f64()?) * 100.0).round() / 100.0);
    Some([d(0)?, d(1)?, d(2)?])
}

fn mm3(v: &Value) -> f64 {
    (v.as_f64().unwrap_or(0.0)).round()
}

/// What changed from the generated tray `a` to the edited design `b`.
fn diff(fa: &BTreeMap<u64, Value>, ma: &Value, fb: &BTreeMap<u64, Value>, mb: &Value) -> Value {
    let mut lines = vec![];
    let (mut added, mut removed, mut changed) = (vec![], vec![], vec![]);
    for (id, f) in fb.iter().filter(|(id, _)| !fa.contains_key(id)) {
        lines.push(format!("added {}", describe(f)));
        added.push(json!({ "id": id, "name": f["name"], "type": f["type"], "text": describe(f) }));
    }
    for (id, f) in fa.iter().filter(|(id, _)| !fb.contains_key(id)) {
        lines.push(format!("removed {}", describe(f)));
        removed.push(json!({ "id": id, "name": f["name"], "type": f["type"], "text": describe(f) }));
    }
    for (id, a) in fa {
        let Some(b) = fb.get(id) else { continue };
        let fields = field_changes(a, b);
        if fields.is_empty() {
            continue;
        }
        let what: Vec<String> = fields.iter().map(|c| format!("{} {} → {}", c["field"].as_str().unwrap_or(""), c["from"].as_str().unwrap_or(""), c["to"].as_str().unwrap_or(""))).collect();
        lines.push(format!("changed {}: {}", describe(a), what.join("; ")));
        changed.push(json!({ "id": id, "name": b["name"], "type": b["type"], "fields": fields }));
    }
    let (pa, pb) = (by_name(ma, "params"), by_name(mb, "params"));
    let mut params = vec![];
    for k in pa.keys().chain(pb.keys()).collect::<std::collections::BTreeSet<_>>() {
        let (x, y) = (pa.get(k), pb.get(k));
        if x != y {
            let show = |p: Option<&Value>| p.map(|p| short(p.get("expression").or(p.get("value")).unwrap_or(p))).unwrap_or("—".into());
            lines.push(format!("parameter {k}: {} → {}", show(x), show(y)));
            params.push(json!({ "name": k, "from": show(x), "to": show(y) }));
        }
    }
    let (ba, bb) = (by_name(ma, "bodies"), by_name(mb, "bodies"));
    let mut bodies = vec![];
    for k in ba.keys().chain(bb.keys()).collect::<std::collections::BTreeSet<_>>() {
        let (x, y) = (ba.get(k), bb.get(k));
        let row = json!({
            "name": k,
            "volume_mm3": [x.map(|b| mm3(&b["volume_mm3"])), y.map(|b| mm3(&b["volume_mm3"]))],
            "size": [x.and_then(size), y.and_then(size)],
        });
        match (x, y) {
            (None, Some(b)) => lines.push(format!("new body {k}: {} mm³, {}", mm3(&b["volume_mm3"]), fmt_size(size(b)))),
            (Some(_), None) => lines.push(format!("body {k} is gone")),
            (Some(a), Some(b)) => {
                let (va, vb) = (mm3(&a["volume_mm3"]), mm3(&b["volume_mm3"]));
                let (sa, sb) = (size(a), size(b));
                if va != vb || sa != sb {
                    let pct = if va > 0.0 { format!(" ({:+.1}%)", (vb - va) / va * 100.0) } else { String::new() };
                    let sz = if sa != sb { format!("; size {} → {}", fmt_size(sa), fmt_size(sb)) } else { String::new() };
                    lines.push(format!("body {k}: volume {va} → {vb} mm³{pct}{sz}"));
                }
            }
            _ => {}
        }
        bodies.push(row);
    }
    if lines.is_empty() {
        lines.push("no change from the generated tray".into());
    }
    json!({ "added": added, "removed": removed, "changed": changed, "params": params, "bodies": bodies, "summary": lines })
}

fn fmt_size(s: Option<[f64; 3]>) -> String {
    s.map(|[x, y, z]| format!("{x} × {y} × {z} mm")).unwrap_or_else(|| "?".into())
}

/// Keep an edited design of tray `t`, compare it with the tray as generated, picture both, and
/// keep the comparison beside it (`edits/<tray>.json`).
pub async fn observe(clip: &Path, games: &Path, slug: &str, layout: &Layout, t: &Tray, solidcraft: &str, design: &[u8]) -> Result<Value, Reason> {
    if !design.starts_with(b"PK") {
        return Err(Reason::new("bad_design", "not a SolidCraft design (.sc): expected a zip container"));
    }
    let dir = edits_dir(games, slug);
    std::fs::create_dir_all(&dir).map_err(|e| Reason::new("io", e.to_string()))?;
    let edited = dir.join(format!("{}.sc", t.id));
    std::fs::write(&edited, design).map_err(|e| Reason::new("io", e.to_string()))?;
    let cache = games.join(slug).join(".build").join("edits").join(crate::geometry::edit_key(t, solidcraft));
    let base = base(clip, layout, t, &cache).await?;
    let work = cache.join(format!("edited-{}", crate::now_ms()));
    std::fs::create_dir_all(&work).map_err(|e| Reason::new("io", e.to_string()))?;
    // The generated tray's reads and picture are kept with its design; the edit's are fresh.
    let (base_png, edited_png) = (cache.join("base.png"), work.join("edited.png"));
    let (ra, rb, pa, pb) = tokio::join!(
        read_design(clip, &cache, &base, "base"),
        read_design(clip, &work, &edited, "edited"),
        picture(clip, &cache, &base, &base_png),
        picture(clip, &work, &edited, &edited_png)
    );
    let shots = pa.and(pb).and_then(|()| {
        let io = |e: std::io::Error| Reason::new("io", e.to_string());
        let pics = pictures_dir(games, slug);
        std::fs::create_dir_all(&pics).map_err(io)?;
        std::fs::copy(&base_png, pics.join(format!("{}.generated.png", t.id))).map_err(io)?;
        std::fs::copy(&edited_png, pics.join(format!("{}.png", t.id))).map_err(io)?;
        Ok(())
    });
    let _ = std::fs::remove_dir_all(&work);
    let ((fa, ma), (fb, mb)) = (ra?, rb?);
    let mut d = diff(&fa, &ma, &fb, &mb);
    // The features it added, as steps that put them on a tray again (`keep_edit`).
    let added: Vec<Value> = fb
        .iter()
        .filter(|(id, _)| !fa.contains_key(id))
        .map(|(_, f)| json!({ "command": "timeline.add", "params": { "feature": f } }))
        .collect();
    std::fs::write(dir.join(format!("{}.added.json", t.id)), serde_json::to_vec(&json!({ "commands": added })).unwrap()).map_err(|e| Reason::new("io", e.to_string()))?;
    d["base_key"] = json!(crate::geometry::edit_key(t, solidcraft));
    d["tray"] = json!(t.id);
    d["saved_ms"] = json!(crate::now_ms());
    d["solidcraft"] = json!(solidcraft);
    if let Err(e) = &shots {
        eprintln!("{slug}/{}: no pictures of the edit: {}", t.id, e.message);
    }
    std::fs::write(dir.join(format!("{}.json", t.id)), serde_json::to_vec_pretty(&d).unwrap()).map_err(|e| Reason::new("io", e.to_string()))?;
    Ok(d)
}

/// Keep the last edit of `tray` as the tray (`keep_edit`): a frozen copy of the design and of
/// its added features, so later edits in SolidCraft don't change what prints until kept again.
pub fn keep(games: &Path, slug: &str, tray: &str) -> Result<crate::spec::KeptEdit, Reason> {
    let dir = edits_dir(games, slug);
    let d = last(games, slug, tray).ok_or_else(|| Reason::new("no_edit", format!("{tray} has no SolidCraft edit to keep: edit it in SolidCraft first")).subject(tray))?;
    let base_key = d["base_key"].as_str().ok_or_else(|| Reason::new("no_edit", "that edit was compared before edits could be kept: save it in SolidCraft again").subject(tray))?;
    let io = |e: std::io::Error| Reason::new("io", e.to_string());
    std::fs::copy(dir.join(format!("{tray}.sc")), dir.join(format!("{tray}.kept.sc"))).map_err(io)?;
    std::fs::copy(dir.join(format!("{tray}.added.json")), dir.join(format!("{tray}.kept.json"))).map_err(io)?;
    let n = |k: &str| d[k].as_array().map_or(0, Vec::len);
    Ok(crate::spec::KeptEdit { base_key: base_key.to_string(), kept_ms: crate::now_ms(), added: n("added"), changed: n("changed") + n("removed") })
}

/// Stop keeping an edit: its frozen copy goes; the last edit and its comparison stay.
pub fn drop_kept(games: &Path, slug: &str, tray: &str) {
    let dir = edits_dir(games, slug);
    for f in [format!("{tray}.kept.sc"), format!("{tray}.kept.json")] {
        let _ = std::fs::remove_file(dir.join(f));
    }
}

/// The kept design of `tray`, if one is kept.
pub fn kept_design(games: &Path, slug: &str, tray: &str) -> Option<PathBuf> {
    Some(edits_dir(games, slug).join(format!("{tray}.kept.sc"))).filter(|p| p.is_file())
}

/// A design's history as steps (`solidcraft-cli script`), which SolidCraft's web app replays
/// into a new design: how a kept edit or a piece opens there again.
pub async fn design_steps(clip: &Path, sc: &Path) -> Result<Vec<Value>, Reason> {
    let work = std::env::temp_dir().join(format!("bid-steps-{}-{}", std::process::id(), crate::now_ms()));
    std::fs::create_dir_all(&work).map_err(|e| Reason::new("io", e.to_string()))?;
    let sc = sc.to_string_lossy().to_string();
    let r = cli(clip, &work, &["script", &sc, "--out", "steps.json"]).await.and_then(|()| read_json(&work.join("steps.json")));
    let _ = std::fs::remove_dir_all(&work);
    let v = r?;
    Ok(v.get("commands").unwrap_or(&v).as_array().cloned().unwrap_or_default())
}

/// Build a tray kept as a SolidCraft edit into `out` (`print/<id>.stl`, `view/<id>.stl`), as the
/// geometry build does for a generated one. While the generated tray is the one the edit was made
/// on, the design prints as it is. Once it has changed, the edit's added features go on the new
/// generated tray (what it changed of the generated features doesn't carry over), with a warning;
/// if they no longer fit, the Err says why and the caller builds the generated tray instead.
pub async fn build_kept(clip: &Path, game: &Path, layout: &Layout, t: &Tray, solidcraft: &str, out: &Path, warnings: &mut Vec<Reason>) -> Result<(), Reason> {
    let k = t.kept_edit.as_ref().ok_or_else(|| Reason::new("no_edit", "no kept edit"))?;
    let io = |e: std::io::Error| Reason::new("io", e.to_string());
    let dir = game.join("edits");
    for sub in ["print", "view"] {
        std::fs::create_dir_all(out.join(sub)).map_err(io)?;
    }
    let cur = crate::geometry::edit_key(t, solidcraft);
    let design = if cur == k.base_key {
        dir.join(format!("{}.kept.sc", t.id))
    } else {
        let mut generated = t.clone();
        generated.kept_edit = None;
        let base = base(clip, layout, &generated, &game.join(".build").join("edits").join(&cur)).await?;
        std::fs::copy(dir.join(format!("{}.kept.json", t.id)), out.join("added.json")).map_err(io)?;
        let base_s = base.to_string_lossy().to_string();
        cli(clip, out, &["run", "added.json", "--in", &base_s, "--quiet", "--save", "replayed.sc"]).await.map_err(|e| {
            Reason::new("edit_not_applied", format!("{} changed since you kept your SolidCraft edit, and the {} feature(s) you added don't fit it now ({}): it prints as generated until you edit it again", t.id, k.added, e.message)).subject(&t.id)
        })?;
        let lost = if k.changed > 0 { format!("; the {} change(s) you made to generated features don't carry over", k.changed) } else { String::new() };
        warnings.push(Reason::new("edit_reapplied", format!("{} changed since you kept your SolidCraft edit: your {} added feature(s) are on the new tray{lost}", t.id, k.added)).subject(&t.id));
        out.join("replayed.sc")
    };
    let p = |sub: &str| out.join(sub).join(format!("{}.stl", t.id)).to_string_lossy().to_string();
    let cmds = json!({ "commands": [
        { "command": "file.save_mesh", "params": { "path": p("print"), "bodies": [t.id] } },
        { "command": "solid.move", "params": { "bodies": [t.id], "translate": t.pos } },
        { "command": "file.save_mesh", "params": { "path": p("view"), "bodies": [t.id] } },
    ] });
    std::fs::write(out.join("kept.json"), serde_json::to_vec(&cmds).unwrap()).map_err(io)?;
    let design_s = design.to_string_lossy().to_string();
    cli(clip, out, &["run", "kept.json", "--in", &design_s, "--quiet"])
        .await
        .map_err(|e| Reason::new("edit_not_applied", format!("{}'s kept SolidCraft edit couldn't be built ({}): it prints as generated", t.id, e.message)).subject(&t.id))?;
    Ok(())
}

/// The last comparison kept for `tray`, if any, with whether its pictures are there (they go
/// when the server restarts).
pub fn last(games: &Path, slug: &str, tray: &str) -> Option<Value> {
    let mut d = read_json(&edits_dir(games, slug).join(format!("{tray}.json"))).ok()?;
    let pics = pictures_dir(games, slug);
    d["pictures"] = json!(pics.join(format!("{tray}.png")).is_file() && pics.join(format!("{tray}.generated.png")).is_file());
    Some(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feats(v: Value) -> BTreeMap<u64, Value> {
        v.as_array().unwrap().iter().map(|f| (f["id"].as_u64().unwrap(), f.clone())).collect()
    }

    #[test]
    fn diff_names_what_was_added_removed_and_changed() {
        let a = feats(json!([
            { "id": 1, "name": "Box1", "type": "box", "operation": "new_body", "length": "40" },
            { "id": 2, "name": "Box2", "type": "box", "operation": "cut", "length": "38", "param_names": ["d4"] },
            { "id": 3, "name": "Cylinder1", "type": "cylinder", "operation": "cut" },
        ]));
        let b = feats(json!([
            { "id": 1, "name": "Box1", "type": "box", "operation": "new_body", "length": "40" },
            { "id": 2, "name": "Box2", "type": "box", "operation": "cut", "length": "36", "param_names": ["d9"] },
            { "id": 4, "name": "Fillet1", "type": "fillet", "radius": "1", "edge_names": ["a", "b", "c"] },
        ]));
        let ma = json!({ "params": [], "bodies": [{ "name": "t", "volume_mm3": 1000.0, "bbox": { "min": [0, 0, 0], "max": [40, 10, 5] } }] });
        let mb = json!({ "params": [{ "name": "wall", "expression": "2" }], "bodies": [{ "name": "t", "volume_mm3": 990.0, "bbox": { "min": [0, 0, 0], "max": [40, 10, 5] } }] });
        let d = diff(&a, &ma, &b, &mb);
        let s: Vec<&str> = d["summary"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(s[0], "added Fillet1 (fillet) radius 1 on 3 edges");
        assert_eq!(s[1], "removed Cylinder1 (cylinder, cut)");
        // param_names moved too, but it is the kernel's bookkeeping, not an edit.
        assert_eq!(s[2], "changed Box2 (box, cut) length 38: length 38 → 36");
        assert_eq!(s[3], "parameter wall: — → 2");
        assert_eq!(s[4], "body t: volume 1000 → 990 mm³ (-1.0%)");
        assert_eq!(s.len(), 5);
    }

    #[test]
    fn an_unchanged_design_says_so() {
        let a = feats(json!([{ "id": 1, "name": "Box1", "type": "box" }]));
        let m = json!({ "params": [], "bodies": [] });
        assert_eq!(diff(&a, &m, &a, &m)["summary"], json!(["no change from the generated tray"]));
    }
}
