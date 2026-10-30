//! Layout → SolidCraft command script → headless `solidcraft-cli run` → meshes.
//!
//! SolidCraft v0.24.1 has no glTF export (import only), so each body is saved as binary STL and
//! this module packs them into one GLB for the browser. Print STLs are saved with each tray at
//! the origin, before it is moved to its place in the box.

use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::Serialize;
use serde_json::{Value, json};

use crate::Reason;
use crate::labels::Label;
use crate::layout::{Layout, Lip, PrintedShell, SLOT_MIN, Shape, Tray};
use crate::spec::LabelMode;

/// Body names of a printed box and its lid; their print STLs are `print/<name>.stl`.
pub const PRINTED_BOX: &str = "box";
pub const PRINTED_LID: &str = "lid";

#[derive(Serialize, Clone, Debug)]
pub struct Build {
    pub dir: PathBuf,
    pub glb: PathBuf,
    pub trays: Vec<String>,
    /// Trays taken from the cache, not rebuilt.
    pub reused: usize,
    pub commands: usize,
    /// Longest solidcraft-cli run (they run in parallel), and the whole build including the GLB.
    pub solidcraft_ms: u128,
    pub total_ms: u128,
    pub warnings: Vec<Reason>,
}

struct Script {
    cmds: Vec<Value>,
}

impl Script {
    fn push(&mut self, command: &str, params: Value) {
        self.cmds.push(json!({ "command": command, "params": params }));
    }
}

/// What one solidcraft-cli run builds: some trays, a printed box, the ghosts, and optionally the
/// 3MF of them all.
struct Part<'a> {
    trays: Vec<&'a Tray>,
    shell: Option<&'a PrintedShell>,
    ghosts: bool,
    threemf: bool,
}

/// A square bar turned 45° about its length (`x` or `y`): `half` from its axis to each corner.
/// Groove ceilings, the click ridge and its notch, and the grip lines are cut or joined with it.
fn diamond(s: &mut Script, name: &str, along_x: bool, length: f64, half: f64, center: [f64; 3]) {
    let side = half * std::f64::consts::SQRT_2;
    let (l, w) = if along_x { (length, side) } else { (side, length) };
    s.push("solid.box", json!({ "length": l, "width": w, "height": side, "center": [0, 0, 0], "operation": "new", "body_name": name }));
    s.push("solid.move", json!({ "bodies": [name], "axis": if along_x { [1, 0, 0] } else { [0, 1, 0] }, "angle": 45 }));
    s.push("solid.move", json!({ "bodies": [name], "translate": center }));
}

/// A 45° lip on a wall's outer face, flush with the top: a block reaching `depth` out (and 0.5 mm
/// into the wall, to join), its underside cut away along the 45° line from the wall face at
/// `depth` below the top to the outer top edge by a turned bar, then joined to the tray. The
/// bar's other faces stay clear of the lip, so the only cut face is the 45° one.
fn lip_script(s: &mut Script, t: &Tray, lip: &Lip, n: usize) {
    let h = t.size[2];
    let (o, d) = (lip.axis, lip.depth);
    let a = 1 - o;
    let name = format!("{}-lip{n}", t.id);
    let bar = format!("{}-lipcut{n}", t.id);
    // The wall face, and the block across it.
    let face = if lip.dir > 0.0 { t.size[o] } else { 0.0 };
    let (o0, o1) = if lip.dir > 0.0 { (face - 0.5, face + d) } else { (face - d, face + 0.5) };
    let mut corner = [0.0, 0.0, h - d];
    let mut size = [0.0, 0.0, d];
    (corner[a], size[a]) = (lip.pos[a], lip.size[a]);
    (corner[o], size[o]) = (o0, o1 - o0);
    s.push("solid.box", json!({ "length": size[0], "width": size[1], "height": size[2], "corner": corner, "operation": "new", "body_name": name }));
    // The bar's upper face toward the wall lies on z = h − d + (distance out from the face).
    let half = d + 3.0;
    let vertex = face - lip.dir * 2.0;
    let mut center = [0.0, 0.0, h - d - 2.0];
    center[a] = lip.pos[a] + lip.size[a] / 2.0;
    center[o] = vertex + lip.dir * half;
    diamond(s, &bar, a == 0, lip.size[a] + 2.0, half, center);
    s.push("solid.combine", json!({ "target": name, "tools": [bar], "operation": "cut" }));
    s.push("solid.combine", json!({ "target": t.id, "tools": [name], "operation": "join" }));
}

/// A label's text as solid, its depth down from its face: cut into the tray (`target`), or as
/// new bodies, one per glyph. `offset` moves it from tray-local to where the tray is.
fn label_script(s: &mut Script, sketch: &str, l: &Label, offset: [f64; 3], target: Option<&str>) {
    let origin = [offset[0], offset[1], offset[2] + l.z];
    s.push("sketch.create", json!({ "plane": { "origin": origin, "x_dir": [1, 0, 0], "y_dir": [0, 1, 0] }, "name": sketch }));
    s.push("sketch.text", json!({ "text": l.text, "at": l.at, "height": l.height, "angle": l.angle, "align": "center" }));
    s.push("sketch.finish", json!({}));
    let mut e = json!({ "sketch": sketch, "distance": l.depth, "direction": "negative", "operation": "new" });
    if let Some(t) = target {
        (e["operation"], e["targets"]) = (json!("cut"), json!([t]));
    }
    s.push("solid.extrude", e);
}

/// The labels whose cuts are filled by a part of their own.
fn inlays(t: &Tray) -> impl Iterator<Item = &Label> {
    t.labels.iter().filter(|l| l.mode == LabelMode::Inlay)
}

/// An inlaid tray's labels, the parts that fill its cuts: `print/<tray>-label.stl` where the
/// tray prints, and `view/<tray>-label.stl` where it sits in the box. Their own run, since
/// SolidCraft makes each glyph a body and names them itself: the document holds only the
/// labels, so saving all of it saves them.
fn fill_script(t: &Tray, dir: &Path) -> Script {
    let mut s = Script { cmds: vec![] };
    for (sub, offset) in [("print", [0.0; 3]), ("view", t.pos)] {
        if sub == "view" {
            s.push("file.new", json!({}));
        }
        for (n, l) in inlays(t).enumerate() {
            label_script(&mut s, &format!("label{n}"), l, offset, None);
        }
        s.push("file.save_mesh", json!({ "path": dir.join(sub).join(format!("{}-label.stl", t.id)).to_string_lossy() }));
    }
    s
}

/// Where a tray's inlaid labels are, once built (`sub`: print or view); none when it has none.
pub fn label_stl(dir: &Path, sub: &str, t: &Tray) -> Option<PathBuf> {
    inlays(t).next().map(|_| dir.join(sub).join(format!("{}-label.stl", t.id)))
}

/// A printed box and its slide-on lid, in box coordinates (see `PrintedShell`). The lid enters
/// at the far end (x = inner length): that end wall stops at the groove floor, and the lid's
/// end covers it. Groove ceilings and the lid's edges are 45°, so neither needs supports; the
/// lid prints top face down, with the click ridge and grip lines facing up.
fn shell_script(s: &mut Script, sh: &PrintedShell, p: &dyn Fn(&str, &str) -> String) {
    let [li, wi, hi] = sh.inner;
    let (w, f, d, g, e) = (sh.wall, sh.floor, sh.groove, sh.gap, sh.detent);
    let top = hi + 2.0 * d;
    let (b, lid) = (PRINTED_BOX, PRINTED_LID);
    s.push("solid.box", json!({ "length": li + 2.0 * w, "width": wi + 2.0 * w, "height": f + top, "corner": [-w, -w, -f], "operation": "new", "body_name": b }));
    s.push("solid.box", json!({ "length": li, "width": wi, "height": top + 1.0, "corner": [0, 0, 0], "operation": "cut", "targets": [b] }));
    // The groove's straight part, all round but the open end, where it runs out through the wall.
    let run = li + w + d + 1.0;
    s.push("solid.box", json!({ "length": run, "width": wi + 2.0 * d, "height": d, "corner": [-d, -d, hi], "operation": "cut", "targets": [b] }));
    // Ceilings: bars whose outer corner is at the groove's depth and whose top corner is above
    // the rim, so no edge lands on the rim; they run a little past each other at the corners.
    let h = d + 0.5;
    let (x0, x1) = (-d - 0.3, li + w + 1.0);
    diamond(s, "groove-1", true, x1 - x0, h, [(x0 + x1) / 2.0, -d + h, hi + d]);
    diamond(s, "groove-2", true, x1 - x0, h, [(x0 + x1) / 2.0, wi + d - h, hi + d]);
    diamond(s, "groove-3", false, wi + 2.0 * d + 0.6, h, [-d + h, wi / 2.0, hi + d]);
    s.push("solid.box", json!({ "length": w + 2.0, "width": wi, "height": 2.0 * d + 1.0, "corner": [li - 1.0, 0, hi], "operation": "new", "body_name": "open-end" }));
    diamond(s, "notch", false, wi + d, e + g, [li + w / 2.0, wi / 2.0, hi]);
    s.push("solid.combine", json!({ "target": b, "tools": ["groove-1", "groove-2", "groove-3", "open-end", "notch"], "operation": "cut" }));

    let [ll, wl, tl] = sh.lid;
    let (lx, ly) = (-d + g, -d + g);
    let lt = hi + tl;
    s.push("solid.box", json!({ "length": ll, "width": wl, "height": tl, "corner": [lx, ly, hi], "operation": "new", "body_name": lid }));
    // Parallel to the groove ceiling, one gap clear of it.
    let ch = d - 2.0 * g + g * std::f64::consts::SQRT_2;
    s.push("solid.chamfer", json!({ "edges": [[lx + ll / 2.0, ly, lt], [lx + ll / 2.0, ly + wl, lt], [lx, ly + wl / 2.0, lt]], "distance": ch, "body": lid }));
    diamond(s, "ridge", false, wi - 4.0, e, [li + w / 2.0, wi / 2.0, hi]);
    s.push("solid.combine", json!({ "target": lid, "tools": ["ridge"], "operation": "join" }));
    let grips: Vec<String> = (0..3).map(|i| format!("grip-{i}")).collect();
    for (i, name) in grips.iter().enumerate() {
        diamond(s, name, false, (wi - 16.0).max(10.0), 0.6, [li + w - 6.0 - 3.0 * i as f64, wi / 2.0, lt]);
    }
    s.push("solid.combine", json!({ "target": lid, "tools": grips, "operation": "cut" }));

    s.push("file.save_mesh", json!({ "path": p("view", b), "bodies": [b] }));
    s.push("file.save_mesh", json!({ "path": p("view", lid), "bodies": [lid] }));
    s.push("solid.move", json!({ "bodies": [b], "translate": [w, w, f] }));
    s.push("file.save_mesh", json!({ "path": p("print", b), "bodies": [b] }));
    // Top face down on the bed.
    s.push("solid.move", json!({ "bodies": [lid], "translate": [-lx, -ly, -hi] }));
    s.push("solid.move", json!({ "bodies": [lid], "axis": [1, 0, 0], "angle": 180 }));
    s.push("solid.move", json!({ "bodies": [lid], "translate": [0, wl, tl] }));
    s.push("file.save_mesh", json!({ "path": p("print", lid), "bodies": [lid] }));
    // Back where they sit closed, for the 3MF.
    s.push("solid.move", json!({ "bodies": [lid], "translate": [0, -wl, -tl] }));
    s.push("solid.move", json!({ "bodies": [lid], "axis": [1, 0, 0], "angle": 180 }));
    s.push("solid.move", json!({ "bodies": [lid], "translate": [lx, ly, hi] }));
    s.push("solid.move", json!({ "bodies": [b], "translate": [-w, -w, -f] }));
}

/// The print STL sizes of a printed box and its lid (the lid lies top down, ridge up).
pub fn shell_print_sizes(sh: &PrintedShell) -> [(&'static str, [f64; 3]); 2] {
    [(PRINTED_BOX, sh.outer), (PRINTED_LID, [sh.lid[0], sh.lid[1], sh.lid[2] + sh.detent])]
}

/// Finger slots in a holder frame: where two bays face each other across a gap at least
/// `SLOT_MIN` wide, the frame between them is cut down to a low bar along their shared run,
/// leaving posts at the corners. Cuts from above only, so no supports.
fn frame_slots(s: &mut Script, t: &Tray) {
    let h = t.size[2];
    let bar = (h * 0.2).max(4.0);
    for a in &t.pockets {
        for b in &t.pockets {
            for axis in 0..2 {
                let o = 1 - axis;
                let gap = (a.pos[axis] + a.size[axis], b.pos[axis]);
                let (lo, hi) = (a.pos[o].max(b.pos[o]), (a.pos[o] + a.size[o]).min(b.pos[o] + b.size[o]));
                // b is the next bay over from a, with nothing between them.
                let between = t.pockets.iter().any(|c| {
                    c.pos[axis] >= gap.0 - 1e-6 && c.pos[axis] + c.size[axis] <= gap.1 + 1e-6 && c.pos[o] < hi && c.pos[o] + c.size[o] > lo
                });
                if gap.1 - gap.0 < SLOT_MIN - 1e-6 || hi - lo < 10.0 || between {
                    continue;
                }
                let m = (hi - lo) * 0.1;
                let mut corner = [0.0, 0.0, bar];
                let mut size = [0.0, 0.0, h - bar + 1.0];
                (corner[axis], size[axis]) = (gap.0 - 0.01, gap.1 - gap.0 + 0.02);
                (corner[o], size[o]) = (lo + m, hi - lo - 2.0 * m);
                s.push("solid.box", json!({ "length": size[0], "width": size[1], "height": size[2], "corner": corner, "operation": "cut", "targets": [t.id] }));
            }
        }
    }
}

/// A notch's floor bites: from each long wall's inner face, a half-circle `floor` deep into the
/// floor, cut through; at most half the slot's width less BITE_STEP. Not wider: one exactly as
/// wide as the slot meets its sides on a tangent, and an elliptical cut, like that tangent,
/// leaves the mesh with open edges (SolidCraft v0.27.1).
const BITE_STEP: f64 = 0.25;

pub fn bite_radius(n: &crate::spec::Notch) -> f64 {
    n.floor.min(n.width / 2.0 - BITE_STEP)
}

fn notch_bites(s: &mut Script, t: &Tray, q: &crate::layout::Pocket, n: crate::spec::Notch) {
    let h = t.size[2];
    let a = if q.size[0] >= q.size[1] { 0 } else { 1 };
    let o = 1 - a;
    let mid = q.pos[a] + q.size[a] / 2.0;
    let r = bite_radius(&n);
    for face in [q.pos[o], q.pos[o] + q.size[o]] {
        let mut c = [0.0; 2];
        (c[a], c[o]) = (mid, face);
        s.push("solid.cylinder", json!({ "diameter": 2.0 * r, "height": h + 2.0, "base": [c[0], c[1], -1.0], "operation": "cut", "targets": [t.id] }));
    }
}

fn script(layout: &Layout, part: &Part, dir: &Path, scoops: bool) -> Script {
    let mut s = Script { cmds: vec![] };
    let p = |sub: &str, name: &str| dir.join(sub).join(format!("{name}.stl")).to_string_lossy().into_owned();
    for t in &part.trays {
        let [l, w, h] = t.size;
        s.push("solid.box", json!({ "length": l, "width": w, "height": h, "corner": [0, 0, 0], "operation": "new", "body_name": t.id }));
        // The outer vertical corners, rounded while the body is still a plain block: four edges
        // picked by a point halfway up each.
        if t.corner_radius > 0.0 {
            let mid = h / 2.0;
            s.push("solid.fillet", json!({ "edges": [[0.0, 0.0, mid], [l, 0.0, mid], [0.0, w, mid], [l, w, mid]], "radius": t.corner_radius, "body": t.id }));
        }
        let mut fillets: Vec<(i64, Vec<Value>)> = vec![];
        for q in &t.pockets {
            let [px, py] = q.pos;
            let [ql, qw] = q.size;
            let z0 = h - q.depth;
            let (cx, cy) = (px + ql / 2.0, py + qw / 2.0);
            match q.shape {
                // A frame's bays go through its floor: the holders stand on the box floor.
                Shape::Rect if t.frame => s.push(
                    "solid.box",
                    json!({ "length": ql, "width": qw, "height": h + 2.0, "corner": [px, py, -1.0], "operation": "cut", "targets": [t.id] }),
                ),
                Shape::Rect => s.push(
                    "solid.box",
                    json!({ "length": ql, "width": qw, "height": q.depth + 1.0, "corner": [px, py, z0], "operation": "cut", "targets": [t.id] }),
                ),
                Shape::Round => s.push(
                    "solid.cylinder",
                    json!({ "diameter": ql, "height": q.depth + 1.0, "base": [cx, cy, z0], "operation": "cut", "targets": [t.id] }),
                ),
            }
            if let Some(r) = q.scoop_radius().filter(|_| scoops) {
                let key = (r * 10.0).floor() as i64;
                let edges = [json!([cx, py, z0]), json!([cx, py + qw, z0]), json!([px, cy, z0]), json!([px + ql, cy, z0])];
                match fillets.iter_mut().find(|f: &&mut (i64, Vec<Value>)| f.0 == key) {
                    Some(f) => f.1.extend(edges),
                    None => fillets.push((key, edges.to_vec())),
                }
            }
            if q.open_ends {
                // A card holder: the short end walls cut away to the floor, out through the
                // tray's edge (unless a neighbouring pocket is in the way). Vertical cuts only,
                // so no supports.
                let long_x = ql >= qw;
                let (edge_far, s0, s1) = if long_x { (l, py, py + qw) } else { (w, px, px + ql) };
                let blocked = |lo: f64, hi: f64| {
                    t.pockets.iter().any(|o| {
                        let (o0, o1, b0, b1) =
                            if long_x { (o.pos[0], o.pos[0] + o.size[0], o.pos[1], o.pos[1] + o.size[1]) } else { (o.pos[1], o.pos[1] + o.size[1], o.pos[0], o.pos[0] + o.size[0]) };
                        o.id != q.id && o1 > lo && o0 < hi && b1 > s0 && b0 < s1
                    })
                };
                let wall = t.wall.max(3.0) + 1.0;
                for side in [0.0, 1.0] {
                    let a0 = if long_x { px + side * ql } else { py + side * qw };
                    let (start, end) = if side == 0.0 {
                        (if blocked(0.0, a0) { a0 - wall } else { -1.0 }, a0 + 1.0)
                    } else {
                        (a0 - 1.0, if blocked(a0, edge_far) { a0 + wall } else { edge_far + 1.0 })
                    };
                    let (corner, size) = if long_x { ([start, s0, z0], [end - start, s1 - s0]) } else { ([s0, start, z0], [s1 - s0, end - start]) };
                    s.push("solid.box", json!({ "length": size[0], "width": size[1], "height": h - z0 + 1.0, "corner": corner, "operation": "cut", "targets": [t.id] }));
                }
            }
            // A window in the floor to push the contents up through.
            if let Some(([wx, wy], [wl, ww])) = q.floor_window_rect() {
                s.push("solid.box", json!({ "length": wl, "width": ww, "height": z0 + 2.0, "corner": [px + wx, py + wy, -1.0], "operation": "cut", "targets": [t.id] }));
            }
            if let Some(d) = q.finger_hole_diameter() {
                s.push("solid.cylinder", json!({ "diameter": d, "height": z0 + 2.0, "base": [cx, cy, -1.0], "operation": "cut", "targets": [t.id] }));
            }
        }
        if t.frame {
            frame_slots(&mut s, t);
        }
        // Labels before the scoops and grab holes, which they're kept clear of: every boolean on a
        // body that already has text in it is slow (each glyph is many faces), so the text goes in
        // once the pockets are cut and as little as possible follows it. Four labels on the
        // Lorcana tokens tray: 3.8 s here, 11.6 s after the fillets (SolidCraft v0.26.0).
        for (n, l) in t.labels.iter().enumerate() {
            label_script(&mut s, &format!("{}-label{n}", t.id), l, [0.0; 3], Some(&t.id));
        }
        // Scoops last, one fillet per radius: each fillet makes every later feature on the body
        // slower, so batching them is several times faster than one per pocket.
        for (r10, edges) in fillets {
            s.push("solid.fillet", json!({ "edges": edges, "radius": r10 as f64 / 10.0, "body": t.id, "fit": "reduce" }));
        }
        // Notches in a pocket's long walls, down to what they keep above its floor: after the
        // scoops too, and vertical, so no supports.
        for q in &t.pockets {
            let Some(n) = q.notch else { continue };
            // With a floor bite, the slot goes on down through the floor.
            let bite = n.floor > 0.0 && n.keep <= 0.0;
            let z = if bite { -1.0 } else { h - q.depth + n.keep };
            let o = if q.size[0] >= q.size[1] { 1 } else { 0 };
            for mut r in t.notch_rects(q) {
                // With a bite, the slot stops at the wall's inner face, where the bite takes over.
                if bite {
                    if r[o + 2] <= q.pos[o] + 1.0 + 1e-6 { r[o + 2] = q.pos[o] } else { r[o] = q.pos[o] + q.size[o] }
                }
                let [x0, y0, x1, y1] = r;
                s.push("solid.box", json!({ "length": x1 - x0, "width": y1 - y0, "height": h - z + 1.0, "corner": [x0, y0, z], "operation": "cut", "targets": [t.id] }));
            }
            if bite {
                notch_bites(&mut s, t, q, n);
            }
        }
        // Holds go on after the scoops, so a notch can't move a scoop's edge.
        for c in t.half_circle.iter().flat_map(|hc| hc.centers) {
            let d = t.half_circle.as_ref().map_or(0.0, |hc| hc.diameter);
            s.push("solid.cylinder", json!({ "diameter": d, "height": h + 2.0, "base": [c[0], c[1], -1.0], "operation": "cut", "targets": [t.id] }));
        }
        for (n, lip) in t.lips.iter().enumerate() {
            lip_script(&mut s, t, lip, n);
        }
        s.push("file.save_mesh", json!({ "path": p("print", &t.id), "bodies": [t.id] }));
        s.push("solid.move", json!({ "bodies": [t.id], "translate": t.pos }));
        s.push("file.save_mesh", json!({ "path": p("view", &t.id), "bodies": [t.id] }));
    }
    // Ghosts: what sits in each pocket, as plain solids in box coordinates.
    for t in layout.trays.iter().filter(|_| part.ghosts) {
        for q in t.pockets.iter().filter(|q| q.items > 0) {
            let name = format!("ghost-{}", q.id);
            let [cl, cw, ch] = q.content;
            // Lifted a hair off the pocket floor so the two surfaces don't z-fight in the viewer.
            let z = t.pos[2] + t.size[2] - q.depth + 0.3;
            let (ox, oy) = (t.pos[0] + q.pos[0], t.pos[1] + q.pos[1]);
            let (cx, cy) = (ox + q.size[0] / 2.0, oy + q.size[1] / 2.0);
            match q.shape {
                Shape::Rect => {
                    let (gl, gw) = if (q.size[0] >= q.size[1]) == (cl >= cw) { (cl, cw) } else { (cw, cl) };
                    s.push("solid.box", json!({ "length": gl, "width": gw, "height": ch, "corner": [cx - gl / 2.0, cy - gw / 2.0, z], "operation": "new", "body_name": name }))
                }
                Shape::Round => {
                    s.push("solid.cylinder", json!({ "diameter": cl, "height": ch, "base": [cx, cy, z], "operation": "new", "body_name": name }))
                }
            }
            s.push("file.save_mesh", json!({ "path": p("view", &name), "bodies": [name] }));
        }
    }
    // What lies loose on top of the insert, as plain slabs.
    for o in layout.on_top.iter().filter(|_| part.ghosts) {
        let name = format!("ghost-top-{}", o.component);
        let [l, w, h] = o.size;
        s.push("solid.box", json!({ "length": l, "width": w, "height": h, "corner": o.pos, "operation": "new", "body_name": name }));
        s.push("file.save_mesh", json!({ "path": p("view", &name), "bodies": [name] }));
    }
    if let Some(sh) = part.shell {
        shell_script(&mut s, sh, &p);
    }
    let mut trays: Vec<&str> = part.trays.iter().map(|t| t.id.as_str()).collect();
    if part.shell.is_some() {
        trays.extend([PRINTED_BOX, PRINTED_LID]);
    }
    if part.threemf && !trays.is_empty() {
        s.push("file.save_mesh", json!({ "path": dir.join("insert.3mf").to_string_lossy(), "bodies": trays }));
    }
    s
}

async fn run_cli(cli: &Path, script: &Script, dir: &Path) -> Result<u128, String> {
    let path = dir.join("script.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&json!({ "commands": script.cmds })).unwrap()).map_err(|e| e.to_string())?;
    let t = Instant::now();
    let out = tokio::process::Command::new(cli)
        .arg("run")
        .arg(&path)
        .arg("--quiet")
        .arg("--allow-errors")
        .current_dir(dir)
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| format!("can't run {}: {e}", cli.display()))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(err.lines().filter(|l| !l.trim().is_empty()).last().unwrap_or("solidcraft-cli failed").to_string());
    }
    Ok(t.elapsed().as_millis())
}

/// Run one part, retrying without scoop fillets if the kernel refuses one (a refused command
/// fails the whole run, even with --allow-errors).
async fn run_part(cli: &Path, layout: &Layout, part: &Part<'_>, dir: &Path, warnings: &mut Vec<Reason>) -> Result<(u128, usize), Reason> {
    for sub in ["print", "view"] {
        std::fs::create_dir_all(dir.join(sub)).map_err(|e| Reason::new("io", e.to_string()))?;
    }
    let s = script(layout, part, dir, true);
    match run_cli(cli, &s, dir).await {
        Ok(ms) => Ok((ms, s.cmds.len())),
        Err(e) if e.contains("solid.fillet") => {
            let ids: Vec<&str> = part.trays.iter().map(|t| t.id.as_str()).collect();
            warnings.push(Reason::new("scoops_dropped", format!("SolidCraft refused a scoop fillet, so {} has none: {e}", ids.join(", "))));
            let s = script(layout, part, dir, false);
            run_cli(cli, &s, dir).await.map(|ms| (ms, s.cmds.len())).map_err(|e| Reason::new("solidcraft_failed", e))
        }
        Err(e) => Err(Reason::new("solidcraft_failed", e)),
    }
}

/// The fills of every inlaid tray in `trays`, each its own run (see `fill_script`).
async fn run_fills(cli: &Path, trays: &[&Tray], dir: &Path) -> Result<(u128, usize), Reason> {
    let (mut ms, mut n) = (0, 0);
    for t in trays.iter().filter(|t| inlays(t).next().is_some()) {
        let s = fill_script(t, dir);
        let fdir = dir.join(format!("fill-{}", t.id));
        std::fs::create_dir_all(&fdir).map_err(|e| Reason::new("io", e.to_string()))?;
        ms += run_cli(cli, &s, &fdir).await.map_err(|e| Reason::new("solidcraft_failed", format!("{}'s labels: {e}", t.id)))?;
        n += s.cmds.len();
    }
    Ok((ms, n))
}

/// One tray's script for SolidCraft's web app (Edit in SolidCraft): the tray alone in print
/// pose, up to its first mesh save, so the design holds only its own history.
pub fn edit_script(layout: &Layout, t: &Tray, scoops: bool) -> Vec<Value> {
    let s = script(layout, &Part { trays: vec![t], shell: None, ghosts: false, threemf: false }, Path::new("."), scoops);
    s.cmds.into_iter().take_while(|c| c["command"] != "file.save_mesh").collect()
}

/// The generated tray's key, whether or not an edit is kept for it: the design an edit is
/// compared with, and made on.
pub fn edit_key(t: &Tray, solidcraft: &str) -> String {
    let mut t = t.clone();
    t.kept_edit = None;
    tray_key(&t, solidcraft)
}

/// A tray's cache key: everything its geometry depends on.
fn tray_key(t: &Tray, solidcraft: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    // Names and requests don't change the geometry: renaming a tray doesn't rebuild it.
    let mut t = t.clone();
    (t.name, t.auto_name, t.request) = (String::new(), String::new(), None);
    serde_json::to_string(&t).unwrap().hash(&mut h);
    solidcraft.hash(&mut h);
    format!("{:016x}", h.finish())
}

fn shell_key(sh: &PrintedShell, solidcraft: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let mut sh = sh.clone();
    sh.why = String::new();
    serde_json::to_string(&sh).unwrap().hash(&mut h);
    solidcraft.hash(&mut h);
    format!("{:016x}", h.finish())
}

/// Build the geometry for a layout into `dir`: one solidcraft-cli process per tray, in parallel,
/// reusing trays from `cache` whose definition hasn't changed, plus one process for the ghosts.
/// `extra`: meshes already in box coordinates (imported pieces), added to the scene as they are.
/// `label_color`: the inlaid labels' filament, for the viewer.
pub async fn build(cli: &Path, solidcraft: &str, layout: &Layout, dir: &Path, cache: &Path, extra: Vec<MeshIn>, label_color: &str) -> Result<Build, Reason> {
    let t0 = Instant::now();
    std::fs::create_dir_all(dir.join("view")).map_err(|e| Reason::new("io", e.to_string()))?;
    if layout.trays.is_empty() && layout.printed.is_none() {
        // Nothing for SolidCraft to make.
        let glb = dir.join("scene.glb");
        write_glb(&extra, &glb).map_err(|e| Reason::new("glb", e))?;
        return Ok(Build { dir: dir.to_path_buf(), glb, trays: vec![], reused: 0, commands: 0, solidcraft_ms: 0, total_ms: t0.elapsed().as_millis(), warnings: vec![] });
    }
    let mut jobs = tokio::task::JoinSet::new();
    let mut reused = 0;
    // What a cached tray's build said (a kept edit put on again, or not), said again on reuse.
    let mut kept_said: Vec<Reason> = vec![];
    for t in &layout.trays {
        let key = tray_key(t, solidcraft);
        let cdir = cache.join(&key);
        if cdir.join("view").join(format!("{}.stl", t.id)).is_file() && label_stl(&cdir, "view", t).is_none_or(|p| p.is_file()) {
            reused += 1;
            if let Ok(s) = std::fs::read(cdir.join("said.json")) {
                kept_said.extend(serde_json::from_slice::<Vec<Reason>>(&s).unwrap_or_default());
            }
            continue;
        }
        let game = cache.parent().and_then(Path::parent).map(Path::to_path_buf).unwrap_or_default();
        let tag = solidcraft.to_string();
        let (cli, layout, tray, cdir) = (cli.to_path_buf(), layout.clone(), t.id.clone(), cdir);
        jobs.spawn(async move {
            // Each job gets its own temp dir: an older build still running may be making the
            // same tray. Whichever finishes first fills the cache; the other's copy is dropped.
            let tmp = cdir.with_extension(format!("tmp-{}-{tray}", crate::now_ms()));
            let t = layout.trays.iter().find(|x| x.id == tray).unwrap();
            let mut w = vec![];
            // A tray kept as the user's SolidCraft edit prints as that, or as generated if it
            // can't be put on the tray as it is now.
            let kept = match &t.kept_edit {
                Some(_) => match crate::edit::build_kept(&cli, &game, &layout, t, &tag, &tmp, &mut w).await {
                    Ok(()) => true,
                    Err(e) => {
                        w.push(e);
                        false
                    }
                },
                None => false,
            };
            let mut r = if kept { Ok((0, 0)) } else { run_part(&cli, &layout, &Part { trays: vec![t], shell: None, ghosts: false, threemf: false }, &tmp, &mut w).await };
            if t.kept_edit.is_some() {
                let said: Vec<&Reason> = w.iter().filter(|x| x.code.starts_with("edit_")).collect();
                let _ = std::fs::write(tmp.join("said.json"), serde_json::to_vec(&said).unwrap());
            }
            if let Ok((ms, n)) = r {
                r = run_fills(&cli, &[t], &tmp).await.map(|(fm, fn_)| (ms + fm, n + fn_));
            }
            if r.is_err() || std::fs::rename(&tmp, &cdir).is_err() {
                let _ = std::fs::remove_dir_all(&tmp);
            }
            r.map(|(ms, n)| (ms, n, w))
        });
    }
    // A printed box is one more job, cached by its own size.
    let shell_dir = layout.printed.as_ref().map(|sh| cache.join(format!("box-{}", shell_key(sh, solidcraft))));
    if let (Some(sh), Some(cdir)) = (&layout.printed, &shell_dir) {
        if cdir.join("view").join(format!("{PRINTED_LID}.stl")).is_file() {
            reused += 1;
        } else {
            let (cli, layout, cdir, sh) = (cli.to_path_buf(), layout.clone(), cdir.clone(), sh.clone());
            jobs.spawn(async move {
                let tmp = cdir.with_extension(format!("tmp-{}", crate::now_ms()));
                let mut w = vec![];
                let r = run_part(&cli, &layout, &Part { trays: vec![], shell: Some(&sh), ghosts: false, threemf: false }, &tmp, &mut w).await;
                if r.is_err() || std::fs::rename(&tmp, &cdir).is_err() {
                    let _ = std::fs::remove_dir_all(&tmp);
                }
                r.map(|(ms, n)| (ms, n, w))
            });
        }
    }
    let mut warnings = kept_said;
    let ghost_dir = dir.join("ghosts");
    // The ghosts run here while the tray jobs run on the runtime.
    let ghosts = Part { trays: vec![], shell: None, ghosts: true, threemf: false };
    let (ghost_ms, mut commands) = run_part(cli, layout, &ghosts, &ghost_dir, &mut warnings).await?;
    let mut solidcraft_ms = ghost_ms;
    while let Some(r) = jobs.join_next().await {
        let (ms, n, w) = r.map_err(|e| Reason::new("internal", e.to_string()))??;
        solidcraft_ms = solidcraft_ms.max(ms);
        commands += n;
        warnings.extend(w);
    }
    let mut meshes = vec![];
    if let Some(cdir) = &shell_dir {
        meshes.push(MeshIn {
            name: PRINTED_BOX.into(),
            stl: cdir.join("view").join(format!("{PRINTED_BOX}.stl")),
            color: "#b9c0ca".into(),
            opacity: 1.0,
            extras: json!({ "kind": "printed_box" }),
        });
        meshes.push(MeshIn {
            name: PRINTED_LID.into(),
            stl: cdir.join("view").join(format!("{PRINTED_LID}.stl")),
            color: "#8d99a8".into(),
            opacity: 1.0,
            extras: json!({ "kind": "printed_lid" }),
        });
    }
    for t in &layout.trays {
        meshes.push(MeshIn {
            name: t.id.clone(),
            stl: cache.join(tray_key(t, solidcraft)).join("view").join(format!("{}.stl", t.id)),
            color: t.color.clone(),
            opacity: 1.0,
            extras: json!({ "kind": "tray", "tray": t.id, "layer": t.layer, "name": t.name }),
        });
        if let Some(stl) = label_stl(&cache.join(tray_key(t, solidcraft)), "view", t) {
            meshes.push(MeshIn {
                name: format!("{}-label", t.id),
                stl,
                color: label_color.into(),
                opacity: 1.0,
                extras: json!({ "kind": "label", "tray": t.id, "layer": t.layer, "name": t.name }),
            });
        }
        for q in t.pockets.iter().filter(|q| q.items > 0) {
            meshes.push(MeshIn {
                name: format!("ghost-{}", q.id),
                stl: ghost_dir.join("view").join(format!("ghost-{}.stl", q.id)),
                color: "#d8dde6".into(),
                opacity: 0.45,
                extras: json!({ "kind": "ghost", "tray": t.id, "layer": t.layer, "component": q.component, "label": q.label }),
            });
        }
    }
    for o in &layout.on_top {
        meshes.push(MeshIn {
            name: format!("ghost-top-{}", o.component),
            stl: ghost_dir.join("view").join(format!("ghost-top-{}.stl", o.component)),
            color: "#d8dde6".into(),
            opacity: 0.45,
            extras: json!({ "kind": "ghost", "tray": "", "layer": layout.layers.len(), "component": o.component, "label": o.label }),
        });
    }
    meshes.extend(extra);
    let glb = dir.join("scene.glb");
    write_glb(&meshes, &glb).map_err(|e| Reason::new("glb", e))?;
    Ok(Build {
        dir: dir.to_path_buf(),
        glb,
        trays: layout.trays.iter().map(|t| t.id.clone()).collect(),
        reused,
        commands,
        solidcraft_ms,
        total_ms: t0.elapsed().as_millis(),
        warnings,
    })
}

/// Everything in one run, for export: print STLs per tray and the 3MF of the whole insert, then
/// the inlaid labels' parts.
pub async fn build_full(cli: &Path, layout: &Layout, dir: &Path) -> Result<(u128, Vec<Reason>), Reason> {
    let mut warnings = vec![];
    let part = Part { trays: layout.trays.iter().collect(), shell: layout.printed.as_ref(), ghosts: false, threemf: true };
    let (ms, _) = run_part(cli, layout, &part, dir, &mut warnings).await?;
    let (fill_ms, _) = run_fills(cli, &part.trays, dir).await?;
    Ok((ms + fill_ms, warnings))
}

pub struct MeshIn {
    pub name: String,
    pub stl: PathBuf,
    pub color: String,
    pub opacity: f64,
    pub extras: Value,
}

/// A printable STL: closed (every edge shared by exactly two triangles), its bounding box
/// matches the tray's size with its min corner at the origin, and nothing needs supports: no
/// downward-facing surface off the bed is flatter than `max_overhang_deg` from horizontal.
/// A tray's print, checked against its footprint and any lips outside it.
pub fn check_tray_stl(path: &Path, t: &Tray, max_overhang_deg: f64) -> Result<usize, String> {
    let (lo, hi) = t.print_bounds();
    check_stl_in(path, lo, hi, max_overhang_deg)
}

pub fn check_stl(path: &Path, size: [f64; 3], max_overhang_deg: f64) -> Result<usize, String> {
    check_stl_in(path, [0.0; 3], size, max_overhang_deg)
}

fn check_stl_in(path: &Path, want_lo: [f64; 3], want_hi: [f64; 3], max_overhang_deg: f64) -> Result<usize, String> {
    let tris = read_stl(path)?;
    if tris.is_empty() {
        return Err(format!("{}: no triangles", path.display()));
    }
    let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
    for p in &tris {
        for i in 0..3 {
            lo[i] = lo[i].min(p[i] as f64);
            hi[i] = hi[i].max(p[i] as f64);
        }
    }
    let (over_area, over_z) = overhang(&tris, max_overhang_deg);
    if over_area > 0.5 {
        return Err(format!(
            "{}: {over_area:.0} mm² of overhang flatter than {max_overhang_deg}° (lowest at z = {over_z:.1} mm) would need supports",
            path.display()
        ));
    }
    let open = open_edges(&tris);
    if open > 0 {
        return Err(format!("{}: {open} edges not shared by exactly two triangles", path.display()));
    }
    for i in 0..3 {
        if (lo[i] - want_lo[i]).abs() > 0.05 || (hi[i] - want_hi[i]).abs() > 0.05 {
            return Err(format!("{}: bounds {lo:?}..{hi:?}, expected {want_lo:?}..{want_hi:?}", path.display()));
        }
    }
    Ok(tris.len() / 3)
}

/// Edges not shared by exactly two triangles: none in a closed mesh.
fn open_edges(tris: &[[f32; 3]]) -> usize {
    // Closed either way counts: with the coordinates as written (the kernel writes a shared
    // corner identically, so this is exact), or rounded to a micron (for corners that differ
    // in the last bits). Each alone misreads some meshes: where a lip's top meets a wall's,
    // corners 0.2 µm apart merge at a micron and leave fans of slivers looking open.
    open_edges_at(tris, None).min(open_edges_at(tris, Some(1e3)))
}

fn open_edges_at(tris: &[[f32; 3]], scale: Option<f64>) -> usize {
    let q = |p: &[f32; 3]| p.map(|c| match scale {
        Some(k) => (c as f64 * k).round() as i64,
        None => c.to_bits() as i64,
    });
    let mut edges = std::collections::HashMap::new();
    for t in tris.chunks(3) {
        let v = [q(&t[0]), q(&t[1]), q(&t[2])];
        // A sliver that collapses at this rounding: its neighbours close the surface without it.
        if v[0] == v[1] || v[1] == v[2] || v[0] == v[2] {
            continue;
        }
        for k in 0..3 {
            let (a, b) = (v[k], v[(k + 1) % 3]);
            *edges.entry(if a < b { (a, b) } else { (b, a) }).or_insert(0u32) += 1;
        }
    }
    edges.values().filter(|&&n| n != 2).count()
}

/// An inlaid label's part: closed, and lying in its tray's cut, so within the tray's print
/// bounds. Not checked for overhang: it stands on the cut's floor, printed with the tray.
pub fn check_label_stl(path: &Path, t: &Tray) -> Result<usize, String> {
    let tris = read_stl(path)?;
    if tris.is_empty() {
        return Err(format!("{}: no triangles", path.display()));
    }
    let open = open_edges(&tris);
    if open > 0 {
        return Err(format!("{}: {open} edges not shared by exactly two triangles", path.display()));
    }
    let (lo, hi) = t.print_bounds();
    if let Some(p) = tris.iter().find(|p| (0..3).any(|i| (p[i] as f64) < lo[i] - 0.05 || (p[i] as f64) > hi[i] + 0.05)) {
        return Err(format!("{}: reaches {p:?}, outside its tray", path.display()));
    }
    Ok(tris.len() / 3)
}

/// Downward-facing area off the bed flatter than `max_overhang_deg` from horizontal, mm², and the
/// lowest z it starts at.
fn overhang(tris: &[[f32; 3]], max_overhang_deg: f64) -> (f64, f64) {
    // A face tilted θ from horizontal, facing down, has normal z = −cos θ.
    let limit = -max_overhang_deg.to_radians().cos();
    let (mut over_area, mut over_z) = (0.0f64, f64::MAX);
    for t in tris.chunks(3) {
        let p = |i: usize| [t[i][0] as f64, t[i][1] as f64, t[i][2] as f64];
        let (a, b, c) = (p(0), p(1), p(2));
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        let low = a[2].min(b[2]).min(c[2]);
        // 1e-4 of slack (about 0.01°): an exact 45° face's f32 normal can be 1e-5 past the limit.
        if len > 1e-12 && n[2] / len < limit - 1e-4 && low > 0.01 {
            over_area += len / 2.0;
            over_z = over_z.min(low);
        }
    }
    (over_area, over_z)
}

/// A print STL as it lies on its plate (`m`, from `plates`), checked for supports: the overhang
/// area in mm² past 0.5, or none.
pub fn plate_overhang(path: &Path, m: &crate::plates::M4, max_overhang_deg: f64) -> Result<Option<f64>, String> {
    let tris: Vec<[f32; 3]> = read_stl(path)?.into_iter().map(|p| crate::plates::apply(m, p.map(f64::from)).map(|c| c as f32)).collect();
    let (area, _) = overhang(&tris, max_overhang_deg);
    Ok((area > 0.5).then_some(area))
}

/// Where a view build left a piece's print STL (`part`: a tray id, or the printed box or lid).
pub fn cached_print_stl(cache: &Path, solidcraft: &str, layout: &Layout, part: &str) -> Option<PathBuf> {
    let dir = match layout.trays.iter().find(|t| t.id == part) {
        Some(t) => cache.join(tray_key(t, solidcraft)),
        None => cache.join(format!("box-{}", shell_key(layout.printed.as_ref()?, solidcraft))),
    };
    Some(dir.join("print").join(format!("{part}.stl"))).filter(|p| p.is_file())
}

/// A piece's print STLs part by part, as its plate piece lists them: a tray, then its inlaid
/// labels; a printed box or lid alone. Empty until the view build has made them.
pub fn cached_print_parts(cache: &Path, solidcraft: &str, layout: &Layout, part: &str) -> Vec<PathBuf> {
    let Some(main) = cached_print_stl(cache, solidcraft, layout, part) else { return vec![] };
    let label = layout.trays.iter().find(|t| t.id == part).and_then(|t| label_stl(&cache.join(tray_key(t, solidcraft)), "print", t));
    std::iter::once(main).chain(label.filter(|p| p.is_file())).collect()
}

/// Binary STL → flat triangle list of positions.
pub fn read_stl(path: &Path) -> Result<Vec<[f32; 3]>, String> {
    let b = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if b.len() < 84 {
        return Err(format!("{}: too short for binary STL", path.display()));
    }
    let n = u32::from_le_bytes(b[80..84].try_into().unwrap()) as usize;
    if b.len() < 84 + n * 50 {
        return Err(format!("{}: truncated", path.display()));
    }
    let f = |o: usize| f32::from_le_bytes(b[o..o + 4].try_into().unwrap());
    let mut v = Vec::with_capacity(n * 3);
    for i in 0..n {
        let o = 84 + i * 50 + 12;
        for k in 0..3 {
            let p = o + k * 12;
            v.push([f(p), f(p + 4), f(p + 8)]);
        }
    }
    Ok(v)
}

fn hex_rgb(s: &str) -> [f64; 3] {
    let h = s.trim_start_matches('#');
    let c = |i: usize| u8::from_str_radix(h.get(i..i + 2).unwrap_or("80"), 16).unwrap_or(128) as f64 / 255.0;
    // glTF colour factors are linear.
    let lin = |c: f64| if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
    [lin(c(0)), lin(c(2)), lin(c(4))]
}

pub fn write_glb(meshes: &[MeshIn], out: &Path) -> Result<(), String> {
    let mut bin: Vec<u8> = vec![];
    let (mut nodes, mut gmeshes, mut materials, mut accessors, mut views) = (vec![], vec![], vec![], vec![], vec![]);
    for (i, m) in meshes.iter().enumerate() {
        let pos = read_stl(&m.stl)?;
        let mut nrm = Vec::with_capacity(pos.len());
        for t in pos.chunks(3) {
            let (a, b, c) = (t[0], t[1], t[2]);
            let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-20);
            let n = [n[0] / len, n[1] / len, n[2] / len];
            nrm.extend([n, n, n]);
        }
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for p in &pos {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        if pos.is_empty() {
            lo = [0.0; 3];
            hi = [0.0; 3];
        }
        for (data, is_pos) in [(&pos, true), (&nrm, false)] {
            let off = bin.len();
            for p in data.iter() {
                for c in p {
                    bin.extend_from_slice(&c.to_le_bytes());
                }
            }
            views.push(json!({ "buffer": 0, "byteOffset": off, "byteLength": bin.len() - off, "target": 34962 }));
            let mut acc = json!({ "bufferView": views.len() - 1, "componentType": 5126, "count": data.len(), "type": "VEC3" });
            if is_pos {
                acc["min"] = json!(lo);
                acc["max"] = json!(hi);
            }
            accessors.push(acc);
        }
        let [r, g, b] = hex_rgb(&m.color);
        let mut mat = json!({
            "name": m.name,
            "pbrMetallicRoughness": { "baseColorFactor": [r, g, b, m.opacity], "metallicFactor": 0.0, "roughnessFactor": 0.75 },
            "doubleSided": true
        });
        if m.opacity < 1.0 {
            mat["alphaMode"] = json!("BLEND");
        }
        materials.push(mat);
        gmeshes.push(json!({ "name": m.name, "primitives": [{ "attributes": { "POSITION": accessors.len() - 2, "NORMAL": accessors.len() - 1 }, "material": i }] }));
        nodes.push(json!({ "name": m.name, "mesh": i, "extras": m.extras }));
    }
    let doc = json!({
        "asset": { "version": "2.0", "generator": "BoxInsertDesigner (meshes from SolidCraft)" },
        "scene": 0,
        "scenes": [{ "nodes": (0..nodes.len()).collect::<Vec<_>>() }],
        "nodes": nodes, "meshes": gmeshes, "materials": materials,
        "accessors": accessors, "bufferViews": views,
        "buffers": [{ "byteLength": bin.len() }],
    });
    let mut js = serde_json::to_vec(&doc).unwrap();
    while js.len() % 4 != 0 {
        js.push(b' ');
    }
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let total = 12 + 8 + js.len() + if bin.is_empty() { 0 } else { 8 + bin.len() };
    let mut out_b = Vec::with_capacity(total);
    out_b.extend_from_slice(&0x4654_6C67u32.to_le_bytes());
    out_b.extend_from_slice(&2u32.to_le_bytes());
    out_b.extend_from_slice(&(total as u32).to_le_bytes());
    out_b.extend_from_slice(&(js.len() as u32).to_le_bytes());
    out_b.extend_from_slice(&0x4E4F_534Au32.to_le_bytes());
    out_b.extend_from_slice(&js);
    if !bin.is_empty() {
        out_b.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        out_b.extend_from_slice(&0x004E_4942u32.to_le_bytes());
        out_b.extend_from_slice(&bin);
    }
    std::fs::write(out, out_b).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sub_micron_sliver_does_not_open_a_closed_mesh() {
        let (a, b, c, d) = ([0.0f32, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 0.0], [0.0, 0.0, 10.0]);
        let mut tris = vec![a, c, b, a, b, d, a, d, c, b, c, d];
        assert_eq!(open_edges(&tris), 0);
        // A sliver with two corners 0.4 µm apart, as the kernel leaves where a lip meets a wall.
        tris.extend([a, [0.0004, 0.0, 0.0], b]);
        assert_eq!(open_edges(&tris), 0);
        // A face missing is still open, either way it's counted.
        assert_eq!(open_edges(&tris[3..]), 3);
    }

    /// Binary STL of an axis-aligned block, outward-facing triangles.
    fn block(path: &Path, lo: [f32; 3], hi: [f32; 3]) {
        let c = |i: u8| [if i & 1 == 0 { lo[0] } else { hi[0] }, if i & 2 == 0 { lo[1] } else { hi[1] }, if i & 4 == 0 { lo[2] } else { hi[2] }];
        // Faces as corner indices, counter-clockwise seen from outside.
        let quads = [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]];
        let mut b = vec![0u8; 80];
        b.extend_from_slice(&12u32.to_le_bytes());
        for q in quads {
            for t in [[q[0], q[1], q[2]], [q[0], q[2], q[3]]] {
                b.extend_from_slice(&[0u8; 12]);
                for i in t {
                    for v in c(i) {
                        b.extend_from_slice(&v.to_le_bytes());
                    }
                }
                b.extend_from_slice(&[0u8; 2]);
            }
        }
        std::fs::write(path, b).unwrap();
    }

    #[test]
    fn overhang_off_the_bed_is_refused() {
        let dir = std::env::temp_dir().join(format!("bid-overhang-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let on_bed = dir.join("on.stl");
        block(&on_bed, [0.0, 0.0, 0.0], [20.0, 10.0, 5.0]);
        assert!(check_stl(&on_bed, [20.0, 10.0, 5.0], 45.0).is_ok());
        // The same block lifted 5 mm: its underside is a flat ceiling that needs supports.
        let lifted = dir.join("lifted.stl");
        block(&lifted, [0.0, 0.0, 5.0], [20.0, 10.0, 10.0]);
        let e = check_stl(&lifted, [20.0, 10.0, 10.0], 45.0).unwrap_err();
        assert!(e.contains("overhang"), "{e}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
