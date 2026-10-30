//! Print plates: every piece to print (trays, holders, a printed box and its lid) laid out on the
//! printer's bed in print orientation, as many plates as it takes. Pieces the user placed
//! (`spec.plates`, from `set_plate`) keep their plate, turn and flip; the packer fills in the
//! rest, largest first, each on the first plate with room.

use serde::Serialize;

use crate::Reason;
use crate::geometry::{PRINTED_BOX, PRINTED_LID};
use crate::layout::{self, PrintedShell, Tray};
use crate::spec::{PlatePin, Spec};

/// Room kept between pieces on a plate, mm.
pub const GAP: f64 = 5.0;
/// Room kept from an excluded area: Bambu Studio warns of collisions closer than this.
pub const EXCLUDE_CLEAR: f64 = 5.0;
/// The purge tower's footprint, kept free on a plate printing several filaments: Studio's
/// prime_tower_width (35) and brim (3 a side); its depth grows with the purge volumes, so 40 is
/// a generous guess for a few filaments.
pub const TOWER: [f64; 2] = [41.0, 46.0];
pub const TOWER_BRIM: f64 = 3.0;
/// Bambu Studio lays plates out 1.2 beds apart, in rows of ⌈√n⌉ going towards −y (read off its
/// saved projects).
pub const PLATE_STRIDE: f64 = 1.2;

/// A rigid transform, row-major, acting on column vectors.
pub type M4 = [[f64; 4]; 4];

const EPS: f64 = 1e-6;

#[derive(Serialize, Clone, Debug)]
pub struct Piece {
    /// `<box>/<part>`: what `set_plate` takes.
    pub id: String,
    #[serde(rename = "box")]
    pub box_: String,
    /// The tray id, or `printed-box` / `printed-lid`.
    pub part: String,
    pub name: String,
    pub color: String,
    /// tray | frame | holder | printed_box | printed_lid
    pub kind: &'static str,
    /// 1-based.
    pub plate: usize,
    /// Min corner on its plate, bed coordinates, mm.
    pub pos: [f64; 2],
    /// Its size as it lies on the plate.
    pub size: [f64; 3],
    pub rotate: u32,
    pub flip: bool,
    /// Filament slot (1-based) for the piece, and for each named part of it.
    pub filament: usize,
    pub parts: Vec<PartSlot>,
    /// Print it this many times, in these slots (an imported piece with copies); empty: once.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub copies: Vec<CopyNote>,
    /// What the user set for it, if anything.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pin: Option<PlatePin>,
    /// On the bed, clear of the other pieces and the excluded areas, and under the height limit.
    pub fits: bool,
    /// The print STL → this plate's bed coordinates. Column-major, as three.js takes it.
    pub print_to_plate: [f64; 16],
    /// The viewer's mesh (box coordinates) → this plate's bed coordinates. Column-major.
    pub view_to_plate: [f64; 16],
    #[serde(skip)]
    pub print_m: M4,
    #[serde(skip)]
    pub view_to_print: M4,
}

/// One of several made from the same print ("repeat for each colour"), and its slot.
#[derive(Serialize, Clone, Debug)]
pub struct CopyNote {
    pub name: String,
    pub filament: usize,
}

/// A named part of a piece and the filament slot it prints in.
#[derive(Serialize, Clone, Debug)]
pub struct PartSlot {
    pub name: String,
    pub filament: usize,
}

#[derive(Serialize, Clone, Debug)]
pub struct Plates {
    /// Plate names, plate 1 first ("" for none), one per plate.
    pub names: Vec<String>,
    pub filaments: Vec<crate::spec::Filament>,
    pub printer: String,
    pub bed: [f64; 2],
    pub margin: f64,
    pub max_z: f64,
    pub exclude: Vec<[f64; 4]>,
    pub gap: f64,
    pub count: usize,
    /// Each plate's purge tower footprint (brim included), on plates printing more than one
    /// filament; None where it needs none or there's no room (Studio then puts it where it likes).
    pub towers: Vec<Option<[f64; 4]>>,
    pub pieces: Vec<Piece>,
    pub problems: Vec<Reason>,
    /// Pins for pieces the layout no longer has.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub stale: Vec<String>,
}

impl Plates {
    /// Where plate `n` (1-based) sits in a Bambu Studio project, bed origin, mm.
    pub fn plate_origin(&self, n: usize) -> [f64; 2] {
        let cols = (self.count as f64).sqrt().ceil().max(1.0) as usize;
        let (col, row) = ((n - 1) % cols, (n - 1) / cols);
        [col as f64 * self.bed[0] * PLATE_STRIDE, -(row as f64) * self.bed[1] * PLATE_STRIDE]
    }
}

pub fn identity() -> M4 {
    let mut m = [[0.0; 4]; 4];
    for (i, r) in m.iter_mut().enumerate() {
        r[i] = 1.0;
    }
    m
}

pub fn translate(t: [f64; 3]) -> M4 {
    let mut m = identity();
    for i in 0..3 {
        m[i][3] = t[i];
    }
    m
}

/// Turned over about x: (x, y, z) → (x, −y, −z).
fn turn_over() -> M4 {
    let mut m = identity();
    m[1][1] = -1.0;
    m[2][2] = -1.0;
    m
}

/// A quarter-turn multiple about z, exactly.
fn rot_z(deg: u32) -> M4 {
    let (c, s) = match deg % 360 {
        90 => (0.0, 1.0),
        180 => (-1.0, 0.0),
        270 => (0.0, -1.0),
        _ => (1.0, 0.0),
    };
    let mut m = identity();
    (m[0][0], m[0][1], m[1][0], m[1][1]) = (c, -s, s, c);
    m
}

pub fn mul(a: &M4, b: &M4) -> M4 {
    let mut m = [[0.0; 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            m[i][j] = (0..4).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    m
}

/// The inverse of a rigid transform.
pub fn invert(m: &M4) -> M4 {
    let mut r = identity();
    for i in 0..3 {
        for j in 0..3 {
            r[i][j] = m[j][i];
        }
    }
    for i in 0..3 {
        r[i][3] = -(0..3).map(|k| r[i][k] * m[k][3]).sum::<f64>();
    }
    r
}

pub fn apply(m: &M4, p: [f64; 3]) -> [f64; 3] {
    let mut o = [0.0; 3];
    for (i, v) in o.iter_mut().enumerate() {
        *v = m[i][0] * p[0] + m[i][1] * p[1] + m[i][2] * p[2] + m[i][3];
    }
    o
}

fn col_major(m: &M4) -> [f64; 16] {
    let mut o = [0.0; 16];
    for j in 0..4 {
        for i in 0..4 {
            // Exact zeros, not −0, so the JSON reads cleanly.
            o[j * 4 + i] = m[i][j] + 0.0;
        }
    }
    o
}

/// The bounds of box `lo..hi` under `m`.
fn bounds(m: &M4, lo: [f64; 3], hi: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let (mut a, mut b) = ([f64::MAX; 3], [f64::MIN; 3]);
    for k in 0..8 {
        let c = [if k & 1 == 0 { lo[0] } else { hi[0] }, if k & 2 == 0 { lo[1] } else { hi[1] }, if k & 4 == 0 { lo[2] } else { hi[2] }];
        let p = apply(m, c);
        for i in 0..3 {
            a[i] = a[i].min(p[i]);
            b[i] = b[i].max(p[i]);
        }
    }
    (a, b)
}

/// A piece before it's placed: its print bounds and how the viewer's mesh becomes the print.
struct Raw {
    /// Its named parts, with any slot of their own; a generated tray is one part, named as the
    /// piece.
    parts: Vec<(String, Option<usize>)>,
    /// Its own slot when the user hasn't set one.
    filament: usize,
    copies: Vec<CopyNote>,
    id: String,
    box_: String,
    part: String,
    name: String,
    color: String,
    kind: &'static str,
    lo: [f64; 3],
    hi: [f64; 3],
    view_to_print: M4,
}

fn r_name(r: &Raw) -> &str {
    &r.name
}

/// A tray's parts: the tray, and its inlaid labels on the labels' slot.
pub const LABEL_PART: &str = "Label";

fn tray_raw(box_: &str, t: &Tray, label_slot: usize) -> Raw {
    let (lo, hi) = t.print_bounds();
    let kind = if t.frame {
        "frame"
    } else if t.holder_of.is_some() {
        "holder"
    } else {
        "tray"
    };
    Raw {
        parts: std::iter::once((t.name.clone(), None))
            .chain(t.labels.iter().any(|l| l.mode == crate::spec::LabelMode::Inlay).then(|| (LABEL_PART.to_string(), Some(label_slot))))
            .collect(),
        filament: 1,
        copies: vec![],
        id: format!("{box_}/{}", t.id),
        box_: box_.into(),
        part: t.id.clone(),
        name: t.name.clone(),
        color: t.color.clone(),
        kind,
        lo,
        hi,
        // The print STL is the tray before it's moved into the box (geometry.rs).
        view_to_print: translate([-t.pos[0], -t.pos[1], -t.pos[2]]),
    }
}

/// A printed box prints upright from its outer corner; its lid top face down (see `shell_script`).
fn shell_raw(box_: &str, name: &str, sh: &PrintedShell) -> [Raw; 2] {
    let (w, f) = (sh.wall, sh.floor);
    let [ll, wl, tl] = sh.lid;
    let l0 = -sh.groove + sh.gap;
    let lid = mul(&translate([0.0, wl, tl]), &mul(&turn_over(), &translate([-l0, -l0, -sh.inner[2]])));
    [
        Raw {
            parts: vec![(format!("{name} (printed box)"), None)],
            filament: 1,
            copies: vec![],
            id: format!("{box_}/{PRINTED_BOX}"),
            box_: box_.into(),
            part: PRINTED_BOX.into(),
            name: format!("{name} (printed box)"),
            color: "#b9c0ca".into(),
            kind: "printed_box",
            lo: [0.0; 3],
            hi: sh.outer,
            view_to_print: translate([w, w, f]),
        },
        Raw {
            parts: vec![(format!("{name} lid"), None)],
            filament: 1,
            copies: vec![],
            id: format!("{box_}/{PRINTED_LID}"),
            box_: box_.into(),
            part: PRINTED_LID.into(),
            name: format!("{name} lid"),
            color: "#8d99a8".into(),
            kind: "printed_lid",
            lo: [0.0; 3],
            hi: [ll, wl, tl + sh.detent],
            view_to_print: lid,
        },
    ]
}

/// The first spot, lowest row then leftmost, where a `w × d` footprint fits on a plate holding
/// `placed` (each kept `GAP` clear) without touching an excluded area.
fn find_spot(placed: &[[f64; 4]], exclude: &[[f64; 4]], bed: [f64; 2], m: f64, w: f64, d: f64) -> Option<[f64; 2]> {
    find_spot_by(placed, exclude, bed, m, w, d, false)
}

/// As `find_spot`; with `back`, the spot furthest back, then furthest right (the purge tower).
fn find_spot_by(placed: &[[f64; 4]], exclude: &[[f64; 4]], bed: [f64; 2], m: f64, w: f64, d: f64, back: bool) -> Option<[f64; 2]> {
    let mut xs = vec![m, bed[0] - m - w];
    let mut ys = vec![m, bed[1] - m - d];
    for r in placed {
        xs.push(r[2] + GAP);
        ys.push(r[3] + GAP);
        xs.push(r[0] - GAP - w);
        ys.push(r[1] - GAP - d);
    }
    for e in exclude {
        xs.push(e[2].max(m));
        ys.push(e[3].max(m));
    }
    let mut best: Option<[f64; 2]> = None;
    let hit = |a: [f64; 4], b: [f64; 4]| a[0] < b[2] - EPS && b[0] < a[2] - EPS && a[1] < b[3] - EPS && b[1] < a[3] - EPS;
    for &y in &ys {
        for &x in &xs {
            if x + w > bed[0] - m + EPS || y + d > bed[1] - m + EPS {
                continue;
            }
            let r = [x, y, x + w, y + d];
            let grown = [x - GAP, y - GAP, x + w + GAP, y + d + GAP];
            if placed.iter().any(|p| hit(grown, *p)) || exclude.iter().any(|e| hit(r, *e)) {
                continue;
            }
            if x < m - EPS || y < m - EPS {
                continue;
            }
            let better = |b: [f64; 2]| if back { (y, x) > (b[1], b[0]) } else { (y, x) < (b[1], b[0]) };
            if best.is_none_or(better) {
                best = Some([x, y]);
            }
        }
    }
    best
}

/// Every printed piece of every box's chosen layout, on plates.
pub fn arrange(s: &Spec) -> Plates {
    let mut raws = vec![];
    for gb in s.boxes() {
        let lay = layout::chosen(s, &gb);
        for t in &lay.trays {
            raws.push(tray_raw(&gb.id, t, s.label_slot()));
        }
        if let Some(sh) = &lay.printed {
            raws.extend(shell_raw(&gb.id, &gb.name, sh));
        }
    }
    for pc in &s.pieces {
        let copies: Vec<CopyNote> = if pc.copies.len() > 1 {
            pc.copies.iter().enumerate().map(|(i, c)| CopyNote { name: c.name.clone().unwrap_or_else(|| format!("{}", i + 1)), filament: c.filament.unwrap_or(pc.filament) }).collect()
        } else {
            vec![]
        };
        raws.push(Raw {
            parts: pc.parts.iter().map(|p| (p.name.clone(), p.filament)).collect(),
            filament: pc.filament,
            copies,
            id: format!("piece/{}", pc.id),
            box_: pc.box_.clone().unwrap_or_else(|| s.boxes().first().map(|b| b.id.clone()).unwrap_or_default()),
            part: pc.id.clone(),
            name: pc.name.clone(),
            color: s.filament_slots().get(pc.filament - 1).map_or("#9b9ea0".into(), |f| f.color.clone()),
            kind: "piece",
            lo: [0.0; 3],
            hi: pc.size,
            view_to_print: identity(),
        });
    }
    let p = &s.printer;
    let (bed, m) = (p.bed, p.bed_margin);
    let pin_of = |id: &str| s.plates.iter().find(|x| x.piece == id).cloned();
    let stale: Vec<String> = s.plates.iter().filter(|x| !raws.iter().any(|r| r.id == x.piece)).map(|x| x.piece.clone()).collect();

    // Placed exactly first, then pinned to a plate, then the largest footprints.
    let mut order: Vec<usize> = (0..raws.len()).collect();
    let area = |r: &Raw| (r.hi[0] - r.lo[0]) * (r.hi[1] - r.lo[1]);
    let rank = |r: &Raw| match pin_of(&r.id) {
        Some(PlatePin { pos: Some(_), .. }) => 0,
        Some(PlatePin { plate: Some(_), .. }) => 1,
        _ => 2,
    };
    order.sort_by(|&a, &b| rank(&raws[a]).cmp(&rank(&raws[b])).then(area(&raws[b]).total_cmp(&area(&raws[a]))));

    let corners: Vec<[f64; 4]> = p.bed_exclude.iter().map(|e| [e[0] - EXCLUDE_CLEAR, e[1] - EXCLUDE_CLEAR, e[2] + EXCLUDE_CLEAR, e[3] + EXCLUDE_CLEAR]).collect();
    // The purge tower's corner, back right, kept free on every plate: whether a plate needs a
    // tower is known only once it's packed. Pieces keep GAP from it.
    let tower_at = [bed[0] - m - TOWER[0], bed[1] - m - TOWER[1], bed[0] - m, bed[1] - m];
    let tower_keep = [tower_at[0] - GAP, tower_at[1] - GAP, tower_at[2], tower_at[3]];
    // The strip along the front as deep as an excluded area there, and its clearance.
    let mut front_band: Vec<[f64; 4]> = corners.iter().filter(|e| e[1] <= m + EPS).map(|e| [-1.0, -1.0, bed[0] + 1.0, e[3]]).chain(corners.iter().copied()).collect();
    front_band.push(tower_keep);
    let corners_only = corners.clone();
    let mut corners = corners;
    corners.push(tower_keep);
    let mut plates: Vec<Vec<[f64; 4]>> = vec![];
    let mut placed: Vec<Option<Piece>> = (0..raws.len()).map(|_| None).collect();
    let mut problems = vec![];
    for i in order {
        let r = &raws[i];
        let pin = pin_of(&r.id);
        let flip = pin.as_ref().and_then(|x| x.flip).unwrap_or(false);
        let rots: Vec<u32> = match pin.as_ref().and_then(|x| x.rotate) {
            Some(d) => vec![d],
            None => vec![0, 90],
        };
        let orient = |rot: u32| {
            let o = mul(&rot_z(rot), &if flip { turn_over() } else { identity() });
            let (lo, hi) = bounds(&o, r.lo, r.hi);
            (o, lo, [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]])
        };
        // The best spot on plate k: the lowest top edge, then leftmost. Kept out of the front
        // band beside the excluded corner when it can be (`band`): the X1C draws its calibration
        // lines there, so it's a preference, not a ban (2026-10-30). Only the corner itself
        // is off limits, with EXCLUDE_CLEAR round it.
        let try_on = |rects: &[[f64; 4]], band: bool| -> Option<(u32, [f64; 2])> {
            let ex = if band { &front_band } else { &corners };
            rots.iter()
                .filter_map(|&rot| {
                    let (_, _, sz) = orient(rot);
                    find_spot(rects, ex, bed, m, sz[0], sz[1]).map(|at| (rot, at, at[1] + sz[1]))
                })
                .min_by(|a, b| a.2.total_cmp(&b.2).then(a.1[0].total_cmp(&b.1[0])))
                .map(|(rot, at, _)| (rot, at))
        };
        let try_plate = |rects: &[[f64; 4]]| try_on(rects, true).or_else(|| try_on(rects, false));
        let exact = pin.as_ref().and_then(|x| x.pos.zip(x.plate));
        let (k, rot, at, fits) = match pin.as_ref().and_then(|x| x.plate) {
            Some(n) if exact.is_some() => {
                while plates.len() < n {
                    plates.push(vec![]);
                }
                let at = exact.unwrap().0;
                let (_, _, sz) = orient(rots[0]);
                let r = [at[0], at[1], at[0] + sz[0], at[1] + sz[1]];
                let hit = |a: [f64; 4], b: [f64; 4]| a[0] < b[2] - 0.01 && b[0] < a[2] - 0.01 && a[1] < b[3] - 0.01 && b[1] < a[3] - 0.01;
                let on_bed = r[0] >= -EPS && r[1] >= -EPS && r[2] <= bed[0] + EPS && r[3] <= bed[1] + EPS;
                let clear = !plates[n - 1].iter().any(|p| hit(r, *p)) && !p.bed_exclude.iter().any(|e| hit(r, *e));
                if !(on_bed && clear) {
                    let why = if on_bed { "overlaps another piece or the excluded corner" } else { "runs off the bed" };
                    problems.push(Reason::new("plate_full", format!("{} {why} where it's set on plate {n}", r_name(&raws[i]))).subject(&raws[i].id));
                }
                (n - 1, rots[0], at, on_bed && clear)
            }
            Some(n) => {
                while plates.len() < n {
                    plates.push(vec![]);
                }
                match try_plate(&plates[n - 1]) {
                    Some((rot, at)) => (n - 1, rot, at, true),
                    None => {
                        problems.push(Reason::new("plate_full", format!("{} doesn't fit on plate {n} with what's there", r.name)).subject(&r.id));
                        (n - 1, rots[0], [m, m], false)
                    }
                }
            }
            // Clear of the front band on any plate, before using the band on one.
            None => match [true, false].into_iter().find_map(|band| plates.iter().enumerate().find_map(|(k, rects)| try_on(rects, band).map(|(rot, at)| (k, rot, at)))) {
                Some((k, rot, at)) => (k, rot, at, true),
                None => {
                    plates.push(vec![]);
                    let k = plates.len() - 1;
                    match try_plate(&plates[k]) {
                        Some((rot, at)) => (k, rot, at, true),
                        None => {
                            let (_, _, sz) = orient(rots[0]);
                            problems.push(
                                Reason::new("bed_fit", format!("{} ({:.1} × {:.1} mm) doesn't fit the bed", r.name, sz[0], sz[1])).subject(&r.id),
                            );
                            (k, rots[0], [m, m], false)
                        }
                    }
                }
            },
        };
        let (o, lo, sz) = orient(rot);
        let mut fits = fits;
        if sz[2] > p.max_z + EPS {
            problems.push(Reason::new("too_tall", format!("{} is {:.1} mm tall lying like this; the printer takes {:.0}", r.name, sz[2], p.max_z)).subject(&r.id));
            fits = false;
        }
        plates[k].push([at[0], at[1], at[0] + sz[0], at[1] + sz[1]]);
        let filament = pin.as_ref().and_then(|x| x.filament).unwrap_or(r.filament);
        let parts = r
            .parts
            .iter()
            .map(|(n, own)| PartSlot { name: n.clone(), filament: pin.as_ref().and_then(|x| x.parts.get(n).copied()).or(*own).unwrap_or(filament) })
            .collect();
        let print_m = mul(&translate([at[0] - lo[0], at[1] - lo[1], -lo[2]]), &o);
        let view_m = mul(&print_m, &r.view_to_print);
        placed[i] = Some(Piece {
            id: r.id.clone(),
            box_: r.box_.clone(),
            part: r.part.clone(),
            name: r.name.clone(),
            color: r.color.clone(),
            kind: r.kind,
            plate: k + 1,
            pos: at,
            size: sz,
            rotate: rot,
            flip,
            filament,
            parts,
            copies: r.copies.clone(),
            pin,
            fits,
            print_to_plate: col_major(&print_m),
            view_to_plate: col_major(&view_m),
            print_m,
            view_to_print: r.view_to_print,
        });
    }
    let count = plates.len();
    // A purge tower on each plate printing more than one filament: as far back and right as
    // there's room for it.
    let pieces: Vec<Piece> = placed.into_iter().flatten().collect();
    let towers: Vec<Option<[f64; 4]>> = (0..count)
        .map(|k| {
            let mut slots: Vec<usize> = pieces.iter().filter(|x| x.plate == k + 1).flat_map(|x| std::iter::once(x.filament).chain(x.parts.iter().map(|q| q.filament)).chain(x.copies.iter().map(|c| c.filament))).collect();
            slots.sort();
            slots.dedup();
            if slots.len() < 2 {
                return None;
            }
            // Its kept corner, unless a piece set there by hand took it: then the furthest back
            // room left.
            let hit = |a: [f64; 4], b: [f64; 4]| a[0] < b[2] - EPS && b[0] < a[2] - EPS && a[1] < b[3] - EPS && b[1] < a[3] - EPS;
            let spot = if plates[k].iter().any(|r| hit(tower_keep, *r)) {
                find_spot_by(&plates[k], &corners_only, bed, m, TOWER[0], TOWER[1], true)
            } else {
                Some([tower_at[0], tower_at[1]])
            };
            if spot.is_none() {
                problems.push(Reason::new("no_tower_room", format!("plate {} has no room for the purge tower ({} × {} mm): Bambu Studio will place it", k + 1, TOWER[0], TOWER[1])));
            }
            spot.map(|[x, y]| [x, y, x + TOWER[0], y + TOWER[1]])
        })
        .collect();
    Plates {
        names: (0..count).map(|i| s.plate_names.get(i).cloned().unwrap_or_default()).collect(),
        filaments: s.filament_slots(),
        printer: p.name.clone(),
        bed,
        margin: m,
        max_z: p.max_z,
        exclude: p.bed_exclude.clone(),
        gap: GAP,
        count: plates.len(),
        towers,
        pieces,
        problems,
        stale,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Spec {
        crate::sample::spec()
    }

    #[test]
    fn every_piece_lands_on_the_bed_clear_of_the_rest() {
        let s = sample();
        let pl = arrange(&s);
        assert!(!pl.pieces.is_empty());
        // The sample's 250 × 250 tray can't miss the X1C's excluded corner: reported, not placed.
        assert!(pl.problems.iter().all(|p| p.code == "bed_fit"), "{:?}", pl.problems);
        assert_eq!(pl.problems.len(), pl.pieces.iter().filter(|p| !p.fits).count());
        for a in pl.pieces.iter().filter(|p| p.fits) {
            let [x, y] = a.pos;
            assert!(x >= pl.margin - EPS && y >= pl.margin - EPS, "{}: {:?}", a.id, a.pos);
            assert!(x + a.size[0] <= pl.bed[0] - pl.margin + EPS && y + a.size[1] <= pl.bed[1] - pl.margin + EPS, "{}", a.id);
            for e in &pl.exclude {
                assert!(x >= e[2] - EPS || y >= e[3] - EPS, "{} in the excluded corner", a.id);
            }
            for b in pl.pieces.iter().filter(|b| b.plate == a.plate && b.id != a.id && b.fits) {
                let apart = a.pos[0] + a.size[0] + GAP <= b.pos[0] + EPS
                    || b.pos[0] + b.size[0] + GAP <= a.pos[0] + EPS
                    || a.pos[1] + a.size[1] + GAP <= b.pos[1] + EPS
                    || b.pos[1] + b.size[1] + GAP <= a.pos[1] + EPS;
                assert!(apart, "{} and {} touch", a.id, b.id);
            }
        }
    }

    #[test]
    fn the_print_lands_where_the_piece_says() {
        let s = sample();
        let pl = arrange(&s);
        let lays: Vec<_> = s.boxes().iter().map(|gb| (gb.id.clone(), layout::chosen(&s, gb))).collect();
        for p in &pl.pieces {
            let lay = &lays.iter().find(|(id, _)| *id == p.box_).unwrap().1;
            let (lo, hi) = match lay.trays.iter().find(|t| t.id == p.part) {
                Some(t) => t.print_bounds(),
                None => continue,
            };
            let (a, b) = bounds(&p.print_m, lo, hi);
            for i in 0..2 {
                assert!((a[i] - p.pos[i]).abs() < 1e-6 && (b[i] - a[i] - p.size[i]).abs() < 1e-6, "{}", p.id);
            }
            assert!(a[2].abs() < 1e-6, "{} off the bed", p.id);
            // And the viewer's mesh, which is the print moved into the box, lands in the same place.
            let t = lay.trays.iter().find(|t| t.id == p.part).unwrap();
            let (va, _) = bounds(&mul(&col_to_rows(&p.view_to_plate), &translate(t.pos)), lo, hi);
            assert!((va[0] - a[0]).abs() < 1e-6 && (va[1] - a[1]).abs() < 1e-6, "{}", p.id);
        }
    }

    fn col_to_rows(c: &[f64; 16]) -> M4 {
        let mut m = [[0.0; 4]; 4];
        for j in 0..4 {
            for i in 0..4 {
                m[i][j] = c[j * 4 + i];
            }
        }
        m
    }

    #[test]
    fn a_pin_holds_its_plate_turn_and_flip() {
        let mut s = sample();
        let first = arrange(&s).pieces[0].id.clone();
        s.plates.push(PlatePin { piece: first.clone(), plate: Some(2), rotate: Some(90), flip: Some(true), ..Default::default() });
        let pl = arrange(&s);
        let p = pl.pieces.iter().find(|p| p.id == first).unwrap();
        assert_eq!((p.plate, p.rotate, p.flip), (2, 90, true));
        assert!(pl.count >= 2);
        // Turned over: the underside of its floor is now the top.
        let under = apply(&p.print_m, [0.0, 0.0, 0.0]);
        assert!((under[2] - p.size[2]).abs() < 1e-6, "{under:?} vs {:?}", p.size);
    }

    #[test]
    fn rigid_inverse() {
        let m = mul(&translate([3.0, -2.0, 7.0]), &mul(&rot_z(90), &turn_over()));
        let r = mul(&m, &invert(&m));
        for i in 0..4 {
            for j in 0..4 {
                assert!((r[i][j] - if i == j { 1.0 } else { 0.0 }).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn plates_go_in_rows_like_bambu_studio() {
        let mut pl = arrange(&sample());
        pl.count = 3;
        assert_eq!(pl.plate_origin(1), [0.0, 0.0]);
        assert!((pl.plate_origin(2)[0] - 307.2).abs() < 1e-9);
        assert_eq!(pl.plate_origin(3)[0], 0.0);
        assert!((pl.plate_origin(3)[1] + 307.2).abs() < 1e-9);
    }
}
