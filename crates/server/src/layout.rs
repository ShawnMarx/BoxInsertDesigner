//! Layout v1: a deterministic packer for rectangular trays in stacked layers.
//!
//! Claude picks the intent (a strategy, groups, per-tray tweaks); this module does all the
//! maths. Same spec in, same layout out. Every derived number carries a `why`.

use serde::Serialize;

use crate::Reason;
use crate::spec::{BoxSpec, GameBox, Kind, Spec, TokenShape, Tweak};

pub const STRATEGIES: [(&str, &str, &str); 3] = [
    ("by-group", "One tray per group", "Each group (a player, a faction) gets its own tray; ungrouped parts share one."),
    ("by-kind", "One tray per kind", "Cards, tokens, minis and bins each get their own tray."),
    ("fewest-trays", "Fewest trays", "Packs everything into as few trays as fit the bed, deepest pockets first."),
];

const PALETTE: [&str; 10] =
    ["#e4572e", "#29a3a3", "#f2a541", "#4c6ef5", "#7bc950", "#c05299", "#8f6a48", "#3bb0e0", "#d94f70", "#9c89e8"];
/// Leftover room narrower than this goes into the wall; wider becomes a spare compartment.
const SPARE_MIN: f64 = 12.0;
/// A lift-out card holder to its bay in a frame, per side: looser than tray to tray, since it
/// comes out by itself.
const HOLDER_GAP: f64 = 0.3;
/// Finger slots in a holder frame: a gap between bays at least this wide is cut down to a low
/// bar, so fingers reach the holders and decks on either side.
pub const SLOT_MIN: f64 = 6.0;
const EPS: f64 = 1e-6;

#[derive(Serialize, Clone, Debug)]
pub struct Dim {
    pub mm: f64,
    pub why: String,
}

fn dim(mm: f64, why: impl Into<String>) -> Dim {
    Dim { mm: round3(mm), why: why.into() }
}

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// Wall steps the packer works down, one at a time, until the insert fits.
const LEVELS: [&str; 4] = ["normal walls", "minimum dividers", "minimum walls and dividers", "minimum walls, dividers and floor"];

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Shape {
    Rect,
    Round,
}

/// What one component needs from a tray, before any stack is split.
#[derive(Clone, Debug)]
struct Need {
    component: String,
    name: String,
    kind: &'static str,
    group: Option<String>,
    shape: Shape,
    /// Contents footprint (no clearance), long side first.
    content: [f64; 2],
    pocket: [Dim; 2],
    /// Height of everything, as one stack.
    height: Dim,
    /// Items in the stack, and whether the stack may be split into several pockets.
    items: u32,
    splittable: bool,
    /// Separate identical pockets (minis).
    copies: u32,
    finger_hole: bool,
    scoop: bool,
    headroom: f64,
    headroom_why: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct Pocket {
    pub id: String,
    pub component: String,
    pub label: String,
    pub shape: Shape,
    /// Tray-local min corner, mm.
    pub pos: [f64; 2],
    pub size: [f64; 2],
    /// Depth of the cut from the tray top.
    pub depth: f64,
    /// What sits in the pocket: footprint and height, for the ghost.
    pub content: [f64; 3],
    pub items: u32,
    pub finger_hole: bool,
    /// A card holder: no short end walls, a window in the floor (a tray tweak).
    pub open_ends: bool,
    /// A window in the floor to push the contents up through (open ends, a lift-out holder).
    pub floor_window: bool,
    pub scoop: bool,
    /// A slot through both long side walls at the middle (a pocket tweak).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notch: Option<crate::spec::Notch>,
    pub why: PocketWhy,
}

impl Tray {
    /// A pocket's notch as cuts in plan (x0, y0, x1, y1), one per long side: centred along the
    /// pocket, from its edge out through the wall, to the tray's edge or the next pocket.
    pub fn notch_rects(&self, q: &Pocket) -> Vec<[f64; 4]> {
        let Some(n) = q.notch else { return vec![] };
        let a = if q.size[0] >= q.size[1] { 0 } else { 1 };
        let o = 1 - a;
        let mid = q.pos[a] + q.size[a] / 2.0;
        let (a0, a1) = (mid - n.width / 2.0, mid + n.width / 2.0);
        let across = |p: &Pocket| p.id != q.id && p.pos[a] < a1 && p.pos[a] + p.size[a] > a0;
        let (lo, hi) = (q.pos[o], q.pos[o] + q.size[o]);
        let below = self.pockets.iter().filter(|p| across(p) && p.pos[o] + p.size[o] <= lo + 1e-6).map(|p| p.pos[o] + p.size[o]).fold(-1.0, f64::max);
        let above = self.pockets.iter().filter(|p| across(p) && p.pos[o] >= hi - 1e-6).map(|p| p.pos[o]).fold(self.size[o] + 1.0, f64::min);
        [(below, lo + 1.0), (hi - 1.0, above)]
            .into_iter()
            .map(|(b0, b1)| {
                let mut r = [0.0; 4];
                (r[a], r[a + 2], r[o], r[o + 2]) = (a0, a1, b0, b1);
                r
            })
            .collect()
    }
}

impl Pocket {
    /// The radius of a scoop's fillet round the floor, when the pocket has one big enough.
    pub fn scoop_radius(&self) -> Option<f64> {
        let [ql, qw] = self.size;
        let r = (self.depth - 0.5).min(ql.min(qw) / 2.0 - 0.5).min(12.0);
        (self.scoop && self.shape == Shape::Rect && r >= 2.0).then_some(r)
    }

    /// The finger hole through the floor, centred: its diameter.
    pub fn finger_hole_diameter(&self) -> Option<f64> {
        self.finger_hole.then(|| (self.size[0].min(self.size[1]) * 0.5).min(24.0))
    }

    /// The window through the floor, pocket-local: min corner and size.
    pub fn floor_window_rect(&self) -> Option<([f64; 2], [f64; 2])> {
        let [ql, qw] = self.size;
        let rim = (ql.min(qw) / 4.0).min(11.0);
        (self.floor_window && ql.min(qw) - 2.0 * rim >= 10.0).then(|| ([rim, rim], [ql - 2.0 * rim, qw - 2.0 * rim]))
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct PocketWhy {
    pub length: String,
    pub width: String,
    pub depth: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct Tray {
    pub id: String,
    /// The tray's name: the user's (a `name` tweak) or the packer's.
    pub name: String,
    /// The packer's name for it, kept when the user renames it.
    pub auto_name: String,
    /// Changes the user asked for on this tray, for an agent, with their status.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<crate::spec::Request>,
    pub layer: usize,
    /// Min corner in box coordinates (box inner corner at the origin), mm.
    pub pos: [f64; 3],
    pub size: [f64; 3],
    pub wall: f64,
    /// Radius of the four outer vertical corners (style `walls.corner_radius`; see `Walls::corner_radius_for`).
    pub corner_radius: f64,
    pub floor: f64,
    pub color: String,
    pub pockets: Vec<Pocket>,
    pub bed_fit: bool,
    /// A floorless frame of bays, each holding a lift-out holder (the trays `holder_of` it).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub frame: bool,
    /// The frame this holder sits in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub holder_of: Option<String>,
    /// 45° lips outside the footprint to pull it up by (a holder's grip), tray-local.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub lips: Vec<Lip>,
    /// A half-circle notch through one long side.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub half_circle: Option<HalfCircle>,
    /// Stencil labels cut into its top or its pockets' floors (tweaks `label`, `pocket_labels`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<crate::labels::Label>,
    /// Printed as a SolidCraft edit the user kept, not as generated (tweak `edit`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kept_edit: Option<crate::spec::KeptEdit>,
    pub why: TrayWhy,
}

/// A lip at the top of a wall, reaching `depth` out along `axis` (`dir` ±1) over `pos`..`pos +
/// size` (tray-local, outside the footprint). Its underside is 45°, from the wall face at `depth`
/// below the top up to its outer top edge, so it prints without supports.
#[derive(Serialize, Clone, Debug)]
pub struct Lip {
    pub pos: [f64; 2],
    pub size: [f64; 2],
    pub axis: usize,
    pub dir: f64,
    pub depth: f64,
}

/// Grab holes: a pair of half-circle notches facing each other across the tray, cylinders
/// centred on the two long edges, cut top to bottom through solid between pockets.
#[derive(Serialize, Clone, Debug)]
pub struct HalfCircle {
    pub centers: [[f64; 2]; 2],
    pub diameter: f64,
    pub why: String,
}

impl Tray {
    /// What the print takes up, tray-local: the footprint and any lips outside it.
    pub fn print_bounds(&self) -> ([f64; 3], [f64; 3]) {
        let (mut lo, mut hi) = ([0.0; 3], self.size);
        for l in &self.lips {
            for i in 0..2 {
                lo[i] = f64::min(lo[i], l.pos[i]);
                hi[i] = f64::max(hi[i], l.pos[i] + l.size[i]);
            }
        }
        (lo, hi)
    }
}

/// A holder's grip: one 45° lip on the long wall facing the widest finger slot of its frame (a
/// gap of `SLOT_MIN` or more to the next bay), reaching into the slot. Lips facing each other
/// across a slot are staggered (30% and 70% along) and kept inside the run `frame_slots` cuts
/// down to the bar, so the holder lifts straight out.
fn holder_lip(bays: &[([f64; 2], [f64; 2])], k: usize, hs: [f64; 3], st: &crate::style::Grips, notched: bool) -> Option<Lip> {
    let g = HOLDER_GAP;
    let (bp, bs) = bays[k];
    // On a long wall; on an end wall when slots are cut through the long walls (a notch).
    let a = if (hs[0] >= hs[1]) != notched { 0 } else { 1 };
    let o = 1 - a;
    // The nearest bay on each side across `o` that shares a run along `a`.
    let mut near: [Option<(f64, f64, f64)>; 2] = [None, None];
    for (j, (p, s)) in bays.iter().enumerate() {
        let (lo, hi) = (bp[a].max(p[a]), (bp[a] + bs[a]).min(p[a] + s[a]));
        if j == k || hi - lo < 10.0 {
            continue;
        }
        for (side, gap) in [(0, p[o] - (bp[o] + bs[o])), (1, bp[o] - (p[o] + s[o]))] {
            if gap > -EPS && near[side].is_none_or(|n| gap < n.0) {
                near[side] = Some((gap, lo, hi));
            }
        }
    }
    let (side, (gap, lo, hi)) = near.iter().enumerate().filter_map(|(i, n)| n.map(|n| (i, n))).filter(|(_, n)| n.0 >= SLOT_MIN - EPS).max_by(|x, y| x.1 .0.total_cmp(&y.1 .0).then(y.0.cmp(&x.0)))?;
    let dir = if side == 0 { 1.0 } else { -1.0 };
    let depth = st.lip_depth.min(gap + 2.0 * g - 0.6);
    let bar = (hs[2] * 0.2).max(4.0);
    if depth < 1.0 || hs[2] - depth < bar + 1.0 {
        return None;
    }
    // The slot's cut run, holder-local.
    let m = (hi - lo) * 0.1;
    let (run0, run1) = (lo + m - bp[a] - g + 0.5, hi - m - bp[a] - g - 0.5);
    let len = st.lip_length.min(0.35 * hs[a]).min(run1 - run0);
    if len < 5.0 {
        return None;
    }
    let c = (if dir > 0.0 { 0.3 } else { 0.7 } * hs[a]).clamp(run0 + len / 2.0, run1 - len / 2.0);
    let (mut pos, mut size) = ([0.0; 2], [0.0; 2]);
    (pos[a], size[a]) = (round3(c - len / 2.0), round3(len));
    (pos[o], size[o]) = (if dir > 0.0 { hs[o] } else { -depth }, round3(depth));
    Some(Lip { pos, size, axis: o, dir, depth: round3(depth) })
}

/// Grab holes (the standard): a pair of half-circles facing each other across the tray, top
/// to bottom, never into a pocket. Close enough together they make a pinch grip; farther apart,
/// a two-handed lift. They go in a solid band across the tray between two runs of pockets
/// along its length `a` — the one nearest the middle, or just past `want`'s pocket — made by
/// moving the later pockets along into the `extra` room stretching left. When every pocket
/// holds loose pieces, the room still left goes to those pockets rather than an empty spare.
/// Returns the holes, how much of `extra` they used, and whether every pocket is loose.
#[allow(clippy::too_many_arguments)]
fn grab_pair(spec: &Spec, pockets: &mut [Pocket], size: [f64; 2], a: usize, bw: f64, bd: f64, extra: f64, want: &str) -> Result<(HalfCircle, f64, bool), Reason> {
    let d = spec.rules.grips.half_circle;
    let r = d / 2.0;
    let o = 1 - a;
    let no_room = |why: String| Reason::new("grab_holes", why);
    if size[o] - d < 2.0 * bw + 4.0 {
        return Err(no_room(format!("a {:.1} mm wide tray is too narrow for a pair of {d} mm grab holes", size[o])));
    }
    // Runs of pockets that start together along the length.
    let mut starts: Vec<f64> = pockets.iter().map(|p| p.pos[a]).collect();
    starts.sort_by(f64::total_cmp);
    starts.dedup_by(|x, y| (*x - *y).abs() < 0.5);
    let n = starts.len();
    let runs: Vec<usize> = pockets.iter().map(|p| starts.iter().position(|s| (p.pos[a] - s).abs() < 0.5).unwrap_or(0)).collect();
    let loose = pockets.iter().all(|p| spec.component(&p.component).is_some_and(|c| matches!(c.kind, Kind::Bin { .. } | Kind::Tokens { .. })));
    let band = d + 2.0 * bd;
    // The holes between runs k-1 and k (k == n: after the last run), laid out on a copy.
    let place = |k: usize| -> Option<(Vec<Pocket>, f64, f64)> {
        let mut ps = pockets.to_vec();
        let end = |ps: &[Pocket], below: usize| ps.iter().zip(&runs).filter(|(_, i)| **i < below).map(|(p, _)| p.pos[a] + p.size[a]).fold(bw, f64::max);
        let shift = if k < n { (band - (starts[k] - end(&ps, k))).max(0.0) } else { d + bd };
        if shift > extra + EPS {
            return None;
        }
        let step = if loose { (extra - shift) / n as f64 } else { 0.0 };
        for (p, &i) in ps.iter_mut().zip(&runs) {
            p.pos[a] = round3(p.pos[a] + i as f64 * step + if i >= k { shift } else { 0.0 });
            p.size[a] = round3(p.size[a] + step);
        }
        let lo = end(&ps, k);
        let at = if k < n { (lo + ps.iter().zip(&runs).filter(|(_, i)| **i >= k).map(|(p, _)| p.pos[a]).fold(f64::INFINITY, f64::min)) / 2.0 } else { lo + bd + r };
        (at + r <= size[a] - bw + EPS).then_some((ps, round3(at), if loose { extra } else { shift }))
    };
    let (k, why_at) = if want == "middle" {
        let k = (1..=n).filter(|&k| k < n || n == 1).filter_map(|k| place(k).map(|v| (k, (v.1 - size[a] / 2.0).abs()))).min_by(|x, y| x.1.total_cmp(&y.1)).map(|v| v.0);
        (k, "between the pockets nearest the middle".to_string())
    } else {
        let p = pockets.iter().zip(&runs).filter(|(p, _)| p.component == want).max_by(|x, y| (x.0.pos[a] + x.0.size[a]).total_cmp(&(y.0.pos[a] + y.0.size[a])));
        let (p, &i) = p.ok_or_else(|| Reason::new("grab_holes", format!("the tray holds no '{want}' for grab holes to go past")))?;
        (Some(i + 1), format!("just past {}", p.label))
    };
    let (ps, at, used) = k.and_then(place).ok_or_else(|| no_room(format!("no {band:.1} mm of room along the tray for a pair of {d} mm grab holes {why_at}")))?;
    pockets.clone_from_slice(&ps);
    let mut centers = [[0.0; 2]; 2];
    for (c, edge) in centers.iter_mut().zip([0.0, size[o]]) {
        (c[a], c[o]) = (at, round3(edge));
    }
    let grip = if size[o] <= 90.0 { "close enough to pinch" } else { "far enough apart to lift with two hands" };
    let room = if loose && used > EPS { "; the room left goes to the pockets, so no empty spare" } else { "" };
    Ok((
        HalfCircle { centers, diameter: d, why: format!("a pair of {d} mm grab holes (style grips.half_circle) across the tray, {grip}, {why_at}, top to bottom through solid{room}") },
        used,
        loose,
    ))
}

#[derive(Serialize, Clone, Debug)]
pub struct TrayWhy {
    pub height: String,
    pub footprint: String,
    pub walls: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct Layer {
    pub z: f64,
    pub height: f64,
    /// Share of the usable floor the layer's trays cover.
    pub coverage: f64,
}

#[derive(Serialize, Clone, Debug)]
pub struct FitReport {
    pub available_height: Dim,
    /// How far the closed lid rides above the rim: 0 unless the box allows lid lift and the
    /// insert needs it.
    pub lid_lift: Dim,
    pub stack_height: Dim,
    /// available − stack; negative means the lid won't close.
    pub lid_clearance: Dim,
    pub fits_lid: bool,
    pub bed_fit: bool,
    pub tray_count: usize,
    pub layer_count: usize,
    /// Values still to be confirmed: proposed style values, estimated sleeves.
    pub unconfirmed: Vec<String>,
    /// Which step of the wall ladder this layout needed.
    pub walls_level: String,
    pub wall: Dim,
    pub wall_max: Dim,
    pub divider: Dim,
    pub floor: Dim,
}

#[derive(Serialize, Clone, Debug)]
pub struct Layout {
    pub strategy: String,
    pub title: String,
    pub description: String,
    pub ok: bool,
    pub trays: Vec<Tray>,
    pub layers: Vec<Layer>,
    pub report: FitReport,
    /// Reasons this layout doesn't work (nothing placed for that part, or the lid won't close).
    pub problems: Vec<Reason>,
    /// The box itself, when it is designed to print: sized to these trays.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub printed: Option<PrintedShell>,
    /// Works, but worth knowing.
    pub warnings: Vec<Reason>,
    /// What lies loose on top of the insert (as-is components: a board, a rulebook), stacked in
    /// spec order, for the viewer: one more layer, never printed.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub on_top: Vec<OnTop>,
}

#[derive(Serialize, Clone, Debug)]
pub struct OnTop {
    pub component: String,
    pub label: String,
    /// Min corner in box coordinates, and size, mm.
    pub pos: [f64; 3],
    pub size: [f64; 3],
}

/// A box designed to print, in box coordinates (the inside's corner at the origin, so its outside
/// starts at −wall, −wall, −floor). The lid slides in along x from the far end (x = inner length),
/// in 45° dovetail grooves cut into the walls just under the rim, and clicks shut: a ridge under
/// the lid's end drops into a notch across the end wall.
#[derive(Serialize, Clone, Debug)]
pub struct PrintedShell {
    pub inner: [f64; 3],
    pub outer: [f64; 3],
    pub wall: f64,
    pub floor: f64,
    /// How far the groove cuts into each wall; the lid is twice this thick, less the gap.
    pub groove: f64,
    pub gap: f64,
    pub detent: f64,
    /// The lid as it sits closed: length, width, thickness (the ridge under it not included).
    pub lid: [f64; 3],
    pub bed_fit: bool,
    pub why: String,
}

/// How a pass packs: into the box as given, or (for a printed box) loose in the largest box the
/// bed prints to find its size, then again with that search's packing limits and the found size.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Normal,
    Sizing,
    /// The sizing pass's floor and pocket room, so the trays pack the same way.
    Sized { pack: [f64; 2], room: f64 },
}

struct Rules {
    wall: Dim,
    wall_max: Dim,
    divider: Dim,
    floor: Dim,
    heavy_cards: f64,
    layer_h: f64,
    /// Largest tray footprint side the bed takes.
    bed_max: [f64; 2],
    /// Usable box floor and height.
    usable: [f64; 2],
    /// The floor trays are packed into (the usable floor, unless a printed box is being sized),
    /// and the pocket room the depth search starts from.
    pack: [f64; 2],
    pack_room: f64,
    /// Sizing a printed box: no stretching to fill, no raising the top layer.
    tight: bool,
    avail_h: Dim,
    tray_box: f64,
    gap: f64,
}

fn rules(spec: &Spec, gb: &GameBox, level: usize, lift: f64) -> Rules {
    let st = &spec.rules;
    let p = &spec.printer;
    let b = &gb.size;
    let (wn, wm) = (st.walls.normal, st.walls.min);
    let stepped = |what: &str| format!("style minimum {what} {wm} mm, stepped down from {wn} to fit");
    let divider = if level >= 1 { dim(wm, stepped("divider")) } else { dim(wn, format!("style normal wall {wn} mm")) };
    let wall = if level >= 2 { dim(wm, stepped("outer wall")) } else { dim(wn, format!("style normal wall {wn} mm")) };
    let floor = if level >= 3 {
        dim(st.floor.min, format!("style minimum floor {} mm{}, stepped down from {} to fit", st.floor.min, st.mark("floor.min"), st.floor.normal))
    } else {
        dim(st.floor.normal, format!("style normal floor {} mm{}", st.floor.normal, st.mark("floor.normal")))
    };
    let wall_max = dim(
        st.walls.max,
        format!("style max wall {} mm, reinforcing card stacks of {} mm or more{}", st.walls.max, st.heavy_cards_mm, st.mark("heavy_cards_mm")),
    );
    let avail = b.inner[2] + b.lid_depth + lift - b.top_stack;
    let lift_why = if lift > 0.0 { format!(" + lid lift {lift:.1}") } else { String::new() };
    let avail_h = dim(
        avail,
        format!(
            "box inner height {} + lid depth {}{lift_why} − top stack {} ({:?})",
            b.inner[2], b.lid_depth, b.top_stack, b.source
        )
        .to_lowercase(),
    );
    // A printed box is a print too: its trays sit print-to-print in it.
    let tray_box = if gb.printed.is_some() { st.tolerance.print_to_print } else { st.tolerance.print_to_component };
    let usable = [b.inner[0] - 2.0 * tray_box, b.inner[1] - 2.0 * tray_box];
    Rules {
        pack: usable,
        pack_room: avail - floor.mm,
        tight: false,
        wall,
        wall_max,
        divider,
        floor,
        heavy_cards: st.heavy_cards_mm,
        layer_h: p.layer_height,
        bed_max: [p.bed[0] - 2.0 * p.bed_margin, p.bed[1] - 2.0 * p.bed_margin],
        usable,
        avail_h,
        tray_box,
        gap: st.tolerance.print_to_print,
    }
}

impl Rules {
    /// Longest side a tray may have on either axis (the bed is checked for both orientations).
    fn tray_max(&self) -> [f64; 2] {
        let bed_long = self.bed_max[0].max(self.bed_max[1]);
        let bed_short = self.bed_max[0].min(self.bed_max[1]);
        let (ul, uw) = (self.pack[0].max(self.pack[1]), self.pack[0].min(self.pack[1]));
        [ul.min(bed_long), uw.min(bed_short)]
    }
}

fn needs(spec: &Spec, gb: &GameBox) -> Vec<Need> {
    let tol = spec.rules.tolerance.print_to_component;
    let headroom = tol;
    let headroom_why = format!("{tol} mm print-to-component tolerance above the contents (style)");
    spec.packed_in(&gb.id)
        .map(|comp| {
            let (cl, cl_why) = match comp.clearance {
                Some(c) => (c, format!("{c} mm component clearance")),
                None => (tol, format!("{tol} mm print-to-component tolerance (style)")),
            };
            let base = |kind, shape, content: [f64; 2], cl: f64, height: Dim, items, splittable| {
                let (a, b) = if content[0] >= content[1] { (content[0], content[1]) } else { (content[1], content[0]) };
                let why = |v: f64| format!("{v} contents + 2 × {cl_why}");
                Need {
                    component: comp.id.clone(),
                    name: comp.name.clone(),
                    kind,
                    group: comp.group.clone(),
                    shape,
                    content: [a, b],
                    pocket: [dim(a + 2.0 * cl, why(a)), dim(b + 2.0 * cl, why(b))],
                    height,
                    items,
                    splittable,
                    copies: 1,
                    finger_hole: false,
                    scoop: false,
                    headroom,
                    headroom_why: headroom_why.clone(),
                }
            };
            match &comp.kind {
                Kind::Cards { size, count, stack, sleeve } => {
                    let h = match (stack, count) {
                        (Some(s), _) => dim(*s, format!("measured stack {s} mm")),
                        (None, Some(n)) => {
                            let (per, from) = match sleeve.as_ref().and_then(|s| spec.sleeves.iter().find(|x| &x.id == s)) {
                                Some(sl) => (sl.per_card_mm, format!("sleeve '{}' ({:?})", sl.name, sl.source).to_lowercase()),
                                None => (UNSLEEVED_CARD_MM, "unsleeved card default (UNMEASURED)".to_string()),
                            };
                            dim(*n as f64 * per, format!("{n} cards × {per} mm, {from}"))
                        }
                        (None, None) => unreachable!("validated"),
                    };
                    let items = count.unwrap_or(1);
                    let mut n = base("cards", Shape::Rect, *size, cl, h, items, count.is_some());
                    n.finger_hole = true;
                    n
                }
                Kind::Mini { base: d, height, count } => {
                    let mut n = base(
                        "mini",
                        Shape::Round,
                        [*d, *d],
                        cl,
                        dim(*height, format!("mini height {height} mm")),
                        1,
                        false,
                    );
                    n.copies = *count;
                    n
                }
                Kind::Tokens { shape, size, thickness, count } => {
                    let fp = [size[0], *size.get(1).unwrap_or(&size[0])];
                    let sh = if *shape == TokenShape::Round { Shape::Round } else { Shape::Rect };
                    let h = dim(*count as f64 * thickness, format!("{count} tokens × {thickness} mm"));
                    base("tokens", sh, fp, cl, h, *count, true)
                }
                Kind::Bin { size, depth } => {
                    let mut n = base(
                        "bin",
                        Shape::Rect,
                        *size,
                        0.0,
                        dim(*depth, format!("bin depth {depth} mm")),
                        1,
                        false,
                    );
                    n.pocket = [dim(n.content[0], "bin inside length as given"), dim(n.content[1], "bin inside width as given")];
                    n.headroom = 0.0;
                    n.headroom_why = "bin depth is the inside depth".into();
                    n.scoop = true;
                    n
                }
                Kind::AsIs | Kind::Undecided | Kind::Box(_) => unreachable!("packed_in skips these"),
            }
        })
        .collect()
}

pub const UNSLEEVED_CARD_MM: f64 = 0.31;

/// One pocket's worth of a need, before it is placed.
#[derive(Clone, Debug)]
struct Req {
    need: usize,
    label: String,
    l: f64,
    w: f64,
    depth: f64,
    content_h: f64,
    items: u32,
    why_depth: String,
}

/// Split a need into pockets no deeper than `cap` (the room for contents + headroom). A need
/// that can't be split (a mini, a bin) may exceed `cap`, but never `hard`.
fn split(i: usize, n: &Need, cap: f64, cap_why: &str, hard: f64, hard_why: &str) -> Result<Vec<Req>, Reason> {
    let full = n.height.mm + n.headroom;
    let stacks = if full <= cap + EPS || !n.splittable {
        1
    } else {
        let per = n.height.mm / n.items as f64;
        let fit = ((cap - n.headroom) / per).floor() as u32;
        if fit == 0 {
            return Err(Reason::new(
                "too_shallow",
                format!("{}: one item ({per:.2} mm) + headroom doesn't fit {cap_why}", n.name),
            )
            .subject(&n.component));
        }
        n.items.div_ceil(fit)
    };
    if stacks == 1 && full > hard + EPS {
        return Err(Reason::new(
            "too_tall",
            format!("{}: {:.1} mm tall with headroom, more than {hard_why}", n.name, full),
        )
        .subject(&n.component));
    }
    let mut out = vec![];
    for copy in 0..n.copies {
        for s in 0..stacks {
            let items = if stacks == 1 { n.items } else { n.items / stacks + u32::from(s < n.items % stacks) };
            let h = n.height.mm * items as f64 / n.items as f64;
            let label = match (n.copies > 1, stacks > 1) {
                (true, _) => format!("{} {}", n.name, copy + 1),
                (false, true) => format!("{} {}/{}", n.name, s + 1, stacks),
                _ => n.name.clone(),
            };
            let why_depth = if stacks > 1 {
                format!("{items} of {} split over {stacks} stacks to fit {cap_why} → {:.2} mm + {}", n.height.why, h, n.headroom_why)
            } else {
                format!("{} + {}", n.height.why, n.headroom_why)
            };
            out.push(Req {
                need: i,
                label,
                l: n.pocket[0].mm,
                w: n.pocket[1].mm,
                depth: round3(h + n.headroom),
                content_h: h,
                items,
                why_depth,
            });
        }
    }
    Ok(out)
}

/// Pocket rows inside one tray. Returns the placed pockets (pos relative to the tray's inner
/// corner), the inner extent, and whatever didn't fit.
fn pack_rows(reqs: Vec<Req>, max: [f64; 2], div: f64) -> (Vec<(Req, [f64; 2], bool)>, [f64; 2], Vec<Req>) {
    let mut placed = vec![];
    let mut left = vec![];
    let (mut x, mut y, mut row_w, mut used_l) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for r in reqs {
        // Long side along x when it fits, else turned.
        let orients = if r.l <= max[0] + EPS { [(r.l, r.w, false), (r.w, r.l, true)] } else { [(r.w, r.l, true), (r.l, r.w, false)] };
        let mut done = false;
        for (l, w, turned) in orients {
            if l > max[0] + EPS || w > max[1] + EPS {
                continue;
            }
            let at_x = if x == 0.0 { 0.0 } else { x + div };
            if at_x + l <= max[0] + EPS && y + w <= max[1] + EPS {
                placed.push((r.clone(), [at_x, y], turned));
                x = at_x + l;
                row_w = row_w.max(w);
                used_l = used_l.max(x);
                done = true;
                break;
            }
            let ny = if row_w == 0.0 { y } else { y + row_w + div };
            if l <= max[0] + EPS && ny + w <= max[1] + EPS {
                y = ny;
                placed.push((r.clone(), [0.0, y], turned));
                x = l;
                row_w = w;
                used_l = used_l.max(x);
                done = true;
                break;
            }
        }
        if !done {
            left.push(r);
        }
    }
    (placed, [used_l, y + row_w], left)
}

struct Draft {
    id: String,
    name: String,
    tweak: Option<Tweak>,
    wall: f64,
    divider: f64,
    walls_why: String,
    /// A frame of holders: its pockets are bays, grown for the holders.
    holders: bool,
}

pub fn generate_all(spec: &Spec, gb: &GameBox) -> Vec<Layout> {
    STRATEGIES.iter().map(|(s, _, _)| generate(spec, gb, s)).collect()
}

/// The layout chosen for a box.
pub fn chosen(spec: &Spec, gb: &GameBox) -> Layout {
    generate(spec, gb, &gb.layout.strategy)
}

/// The packer packs under a closed lid first. When that fails and the box allows lid lift, it
/// finds the smallest lift (in whole layers, up to the allowance) that fits, so the lid never
/// rides higher than it must. When nothing fits, the attempt at the full allowance is returned.
pub fn generate(spec: &Spec, gb: &GameBox, strategy: &str) -> Layout {
    if gb.printed.is_some() {
        return generate_printed(spec, gb, strategy);
    }
    let closed = search(spec, gb, strategy, 0.0);
    let allow = gb.size.lid_lift;
    if closed.ok || allow <= 0.0 {
        return closed;
    }
    let full = search(spec, gb, strategy, allow);
    if !full.ok {
        return full;
    }
    let step = spec.printer.layer_height;
    let (mut lo, mut hi) = (0usize, (allow / step).ceil() as usize);
    let mut best = full;
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        let l = search(spec, gb, strategy, (mid as f64 * step).min(allow));
        if l.ok {
            hi = mid;
            best = l;
        } else {
            lo = mid;
        }
    }
    best
}

/// The packer searches stack depth: the deepest pockets that let every layer fit under the lid.
/// Deeper pockets mean fewer of them, so it starts at the full room and works down in 2 mm steps.
/// When nothing fits, the full-room attempt is returned with its problems.
fn search(spec: &Spec, gb: &GameBox, strategy: &str, lift: f64) -> Layout {
    let mut first: Option<Layout> = None;
    for level in 0..LEVELS.len() {
        let r = rules(spec, gb, level, lift);
        let mut cap = r.avail_h.mm - r.floor.mm;
        while cap >= 6.0 {
            let l = generate_capped(spec, gb, strategy, cap, level, lift, Mode::Normal);
            if l.ok {
                return l;
            }
            // Shallower pockets only help when the stack or the floor is what doesn't fit.
            let depth_helps = l.problems.iter().all(|p| p.code == "over_lid");
            first.get_or_insert(l);
            if !depth_helps {
                break;
            }
            cap -= 2.0;
        }
    }
    first.expect("at least one attempt")
}

/// One packing attempt at a given pocket depth cap and wall level.
fn generate_capped(spec: &Spec, gb: &GameBox, strategy: &str, global_cap: f64, level: usize, lift: f64, mode: Mode) -> Layout {
    let mut r = rules(spec, gb, level, lift);
    match mode {
        Mode::Normal => {}
        Mode::Sizing => r.tight = true,
        Mode::Sized { pack, room } => (r.pack, r.pack_room) = (pack, room),
    }
    let needs = needs(spec, gb);
    let (_, title, description) = STRATEGIES.iter().find(|(s, _, _)| *s == strategy).copied().unwrap_or(STRATEGIES[0]);
    let mut problems = vec![];
    let mut warnings = vec![];
    let tweak_for = |id: &str| gb.layout.tweaks.iter().find(|t| t.tray == id).cloned();

    // Pocket room: the whole available height less the floor, or the tray's max_depth tweak.
    // Returns (soft cap for splitting stacks, why, hard cap nothing may exceed, why).
    let room = r.pack_room;
    let room_why = format!("the {:.1} mm under the lid less a {} mm floor", room + r.floor.mm, r.floor.mm);
    let cap_for = |t: &Option<Tweak>| -> (f64, String, f64, String) {
        match t.as_ref().and_then(|t| t.max_depth) {
            Some(d) if d < global_cap.min(room) => {
                let why = format!("the tray's max_depth tweak ({d} mm)");
                (d, why.clone(), d, why)
            }
            _ if global_cap < room - EPS => (
                global_cap,
                format!("{global_cap:.0} mm deep pockets, the deepest that let every layer fit under the lid"),
                room,
                room_why.clone(),
            ),
            _ => (room, room_why.clone(), room, room_why.clone()),
        }
    };

    // 1. Assign needs to trays.
    let mut buckets: Vec<(String, String, Vec<usize>)> = vec![];
    let bucket = |id: String, name: String, i: usize, b: &mut Vec<(String, String, Vec<usize>)>| match b.iter_mut().find(|x| x.0 == id) {
        Some(x) => x.2.push(i),
        None => b.push((id, name, vec![i])),
    };
    match strategy {
        "by-kind" => {
            for (i, n) in needs.iter().enumerate() {
                let (id, name) = match n.kind {
                    "cards" => ("cards", "Cards"),
                    "tokens" => ("tokens", "Tokens"),
                    "mini" => ("minis", "Minis"),
                    _ => ("bins", "Bins"),
                };
                bucket(format!("t-{id}"), name.into(), i, &mut buckets);
            }
        }
        "fewest-trays" => {
            for i in 0..needs.len() {
                bucket("t-all".into(), "Tray".into(), i, &mut buckets);
            }
        }
        _ => {
            for g in &spec.groups {
                for (i, n) in needs.iter().enumerate() {
                    if n.group.as_deref() == Some(&g.id) {
                        bucket(format!("t-{}", g.id), g.name.clone(), i, &mut buckets);
                    }
                }
            }
            for (i, n) in needs.iter().enumerate() {
                if n.group.is_none() {
                    bucket("t-shared".into(), "Shared".into(), i, &mut buckets);
                }
            }
        }
    }

    // 2. Split stacks and pack pockets into trays, overflowing into more trays.
    let max = r.tray_max();
    let is_heavy = |q: &Req| needs[q.need].kind == "cards" && q.content_h + EPS >= r.heavy_cards;
    let mut drafts: Vec<(Draft, Vec<(Req, [f64; 2], bool)>, [f64; 2])> = vec![];
    for (bid, bname, idx) in buckets {
        let tweak = if strategy == "fewest-trays" { None } else { tweak_for(&bid) };
        let (cap, cap_why, hard, hard_why) = cap_for(&tweak);
        let mut reqs = vec![];
        for i in idx {
            match split(i, &needs[i], cap, &cap_why, hard, &hard_why) {
                Ok(v) => reqs.extend(v),
                Err(e) => problems.push(e),
            }
        }
        // A frame of holders (a tweak): each pocket's bay grows by a holder's four walls and the gap
        // it lifts out through. Else one tray per pocket (a tweak), so each stack lifts out alone.
        let holders = tweak.as_ref().and_then(|t| t.holders).unwrap_or(false);
        let per_pocket = !holders && tweak.as_ref().and_then(|t| t.per_pocket).unwrap_or(false);
        if holders {
            let hw = spec.rules.walls.min;
            for q in &mut reqs {
                q.l += 2.0 * (hw + HOLDER_GAP);
                q.w += 2.0 * (hw + HOLDER_GAP);
            }
        }
        if strategy == "fewest-trays" {
            reqs.sort_by(|a, b| b.depth.total_cmp(&a.depth).then(b.w.total_cmp(&a.w)).then(b.l.total_cmp(&a.l)));
        } else {
            reqs.sort_by(|a, b| b.w.min(b.l).total_cmp(&a.w.min(a.l)).then(b.l.total_cmp(&a.l)));
        }
        // An order asked for (a tweak) wins; the pockets it doesn't name keep theirs, after.
        if let Some(o) = tweak.as_ref().map(|t| &t.order).filter(|o| !o.is_empty()) {
            reqs.sort_by_key(|q| o.iter().position(|c| *c == needs[q.need].component).unwrap_or(o.len()));
        }
        // Walls for this bucket: an explicit tweak, else max where heavy card stacks may land
        // (decided per tray after packing; the room is planned for the thicker wall).
        let tweaked = tweak.as_ref().and_then(|t| t.walls);
        let divider = tweaked.unwrap_or(r.divider.mm);
        let plan_wall = tweaked.unwrap_or(if !holders && reqs.iter().any(is_heavy) { r.wall_max.mm } else { r.wall.mm });
        let inner_max = [max[0] - 2.0 * plan_wall, max[1] - 2.0 * plan_wall];
        let mut n = 1;
        while !reqs.is_empty() {
            let before = reqs.len();
            let rest = if per_pocket { reqs.split_off(1) } else { vec![] };
            let (placed, extent, left) = pack_rows(reqs, inner_max, divider);
            if placed.is_empty() {
                for q in &left {
                    let nd = &needs[q.need];
                    problems.push(
                        Reason::new(
                            "pocket_too_big",
                            format!(
                                "{}: a {:.1} × {:.1} mm pocket doesn't fit a tray of at most {:.1} × {:.1} mm inside (bed {:?} less margins, box less clearance)",
                                q.label, q.l, q.w, inner_max[0], inner_max[1], spec.printer.bed
                            ),
                        )
                        .subject(&nd.component),
                    );
                }
                if rest.is_empty() {
                    break;
                }
                reqs = rest;
                continue;
            }
            let (id, name) = if strategy == "fewest-trays" {
                (format!("t-{n}"), format!("Tray {n}"))
            } else if per_pocket {
                let id = if n == 1 { bid.clone() } else { format!("{bid}-{n}") };
                (id, placed[0].0.label.clone())
            } else if n == 1 {
                (bid.clone(), bname.clone())
            } else {
                (format!("{bid}-{n}"), format!("{bname} ({n})"))
            };
            let tw = if strategy == "fewest-trays" { tweak_for(&id) } else { tweak.clone() };
            let heavy = placed.iter().any(|(q, _, _)| is_heavy(q));
            let (wall, walls_why) = match tweaked {
                Some(w) => (w, format!("{w} mm walls and dividers, set by a tray tweak")),
                None if holders => (r.wall.mm, format!("frame: outer {}; the holders carry the cards", r.wall.why)),
                None if heavy => (r.wall_max.mm, format!("outer {}; dividers {}", r.wall_max.why, r.divider.why)),
                None => (r.wall.mm, format!("outer {}; dividers {}", r.wall.why, r.divider.why)),
            };
            drafts.push((Draft { id, name, tweak: tw, wall, divider, walls_why, holders }, placed, extent));
            reqs = left;
            reqs.extend(rest);
            n += 1;
            debug_assert!(reqs.len() < before);
        }
    }

    // 3. Trays as blocks: footprint and natural height.
    struct Block {
        draft: Draft,
        placed: Vec<(Req, [f64; 2], bool)>,
        size: [f64; 3],
        height_why: String,
    }
    let snap = |v: f64| (v / r.layer_h - 1e-6).ceil() * r.layer_h;
    let blocks: Vec<Block> = drafts
        .into_iter()
        .map(|(draft, placed, extent)| {
            let deepest = placed.iter().map(|p| p.0.depth).fold(0.0, f64::max);
            let h = round3(snap(r.floor.mm + deepest));
            Block {
                size: [extent[0] + 2.0 * draft.wall, extent[1] + 2.0 * draft.wall, h],
                height_why: format!("{} mm floor + {deepest:.2} mm deepest pocket, up to whole {} mm layers", r.floor.mm, r.layer_h),
                draft,
                placed,
            }
        })
        .collect();

    // 4. Place blocks in layers: tallest first, first layer with room, shelves along y.
    let mut order: Vec<usize> = (0..blocks.len()).collect();
    order.sort_by(|&a, &b| {
        let (x, y) = (&blocks[a].size, &blocks[b].size);
        y[2].total_cmp(&x[2]).then((y[0] * y[1]).total_cmp(&(x[0] * x[1]))).then(blocks[a].draft.id.cmp(&blocks[b].draft.id))
    });
    #[derive(Default)]
    struct Shelf {
        y: f64,
        depth: f64,
        x: f64,
        items: Vec<(usize, bool)>,
    }
    let mut layers: Vec<Vec<Shelf>> = vec![];
    // How many trays share this footprint (one per deck, say), so a new shelf can be laid out
    // for all of them.
    let like = |l: f64, w: f64| {
        let (p, q) = (l.max(w), l.min(w));
        let same = |b: &&Block| (b.size[0].max(b.size[1]) - p).abs() < EPS && (b.size[0].min(b.size[1]) - q).abs() < EPS;
        blocks.iter().filter(same).count() as f64
    };
    let fits = |shelves: &Vec<Shelf>, l: f64, w: f64| -> Option<(Option<usize>, bool)> {
        let mut best: Option<(f64, usize, bool)> = None;
        for (si, s) in shelves.iter().enumerate() {
            for (ll, ww, t) in [(l, w, false), (w, l, true)] {
                let at = if s.x == 0.0 { 0.0 } else { s.x + r.gap };
                if ww <= s.depth + EPS && at + ll <= r.pack[0] + EPS {
                    let waste = s.depth - ww;
                    if best.is_none_or(|b| waste < b.0) {
                        best = Some((waste, si, t));
                    }
                }
            }
        }
        if let Some((_, si, t)) = best {
            return Some((Some(si), t));
        }
        let used = shelves.last().map(|s| s.y + s.depth + r.gap).unwrap_or(0.0);
        // New shelf: the orientation that uses the least depth per tray, counting the trays of
        // this size it could hold side by side (one per deck, say); else the least depth.
        let n = like(l, w);
        let per = |ll: f64, ww: f64| ww / ((r.pack[0] + r.gap + EPS) / (ll + r.gap)).floor().clamp(1.0, n);
        let mut opts = [(l, w, false), (w, l, true)];
        opts.sort_by(|a, b| per(a.0, a.1).total_cmp(&per(b.0, b.1)).then(a.1.total_cmp(&b.1)));
        opts.iter().find(|(ll, ww, _)| *ll <= r.pack[0] + EPS && used + ww <= r.pack[1] + EPS).map(|o| (None, o.2))
    };
    for &bi in &order {
        let [l, w, _] = blocks[bi].size;
        let mut placed = false;
        for li in 0..=layers.len() {
            if li == layers.len() {
                layers.push(vec![]);
            }
            if let Some((shelf, turned)) = fits(&layers[li], l, w) {
                let (ll, ww) = if turned { (w, l) } else { (l, w) };
                let shelves = &mut layers[li];
                let si = match shelf {
                    Some(si) => si,
                    None => {
                        let y = shelves.last().map(|s| s.y + s.depth + r.gap).unwrap_or(0.0);
                        shelves.push(Shelf { y, depth: ww, ..Default::default() });
                        shelves.len() - 1
                    }
                };
                let s = &mut shelves[si];
                s.x = if s.x == 0.0 { ll } else { s.x + r.gap + ll };
                s.items.push((bi, turned));
                placed = true;
                break;
            }
            if layers[li].is_empty() {
                layers.pop();
                break;
            }
        }
        if !placed {
            problems.push(
                Reason::new("tray_too_big", format!("tray {} ({:.1} × {:.1} mm) doesn't fit the box floor", blocks[bi].draft.id, l, w))
                    .subject(&blocks[bi].draft.id),
            );
        }
    }

    // 5. Stretch trays to fill each shelf (up to the bed), turn spare room into compartments,
    //    and emit trays with box coordinates.
    let mut trays: Vec<Tray> = vec![];
    let mut layer_meta: Vec<Layer> = vec![];
    let mut z = 0.0;
    let nl = layers.len();
    for (li, shelves) in layers.iter().enumerate() {
        let mut layer_trays = vec![];
        let ns = shelves.len();
        // The floor left past the last shelf goes to the first shelf with a frame of holders
        // (finger slots between its bays), else to the last shelf; the shelves after it move up.
        let spare_y = shelves.last().map_or(0.0, |s| r.usable[1] - s.y - s.depth);
        let grow = shelves.iter().position(|s| s.items.iter().any(|&(bi, _)| blocks[bi].draft.holders)).unwrap_or(ns.saturating_sub(1));
        for (si, s) in shelves.iter().enumerate() {
            let (depth, y) = match si.cmp(&grow) {
                _ if r.tight => (s.depth, s.y),
                std::cmp::Ordering::Less => (s.depth, s.y),
                std::cmp::Ordering::Equal => (s.depth + spare_y, s.y),
                std::cmp::Ordering::Greater => (s.depth, s.y + spare_y),
            };
            let mut x = 0.0;
            let k = s.items.len();
            for (ii, &(bi, turned)) in s.items.iter().enumerate() {
                let b = &blocks[bi];
                let (l0, w0) = if turned { (b.size[1], b.size[0]) } else { (b.size[0], b.size[1]) };
                let cap_l = if turned { max[1] } else { max[0] };
                let cap_w = if turned { max[0] } else { max[1] };
                let room_l = if ii + 1 == k && !r.tight { r.usable[0] - x } else { l0 };
                let l = room_l.min(cap_l).max(l0);
                let w = depth.min(cap_w).max(w0);
                layer_trays.push((bi, turned, [x, y], [l, w], format!(
                    "{:.1} × {:.1} mm of pockets and walls{}",
                    l0,
                    w0,
                    if (l - l0).abs() > EPS || (w - w0).abs() > EPS {
                        format!(", stretched to {:.1} × {:.1} to fill the box (bed max {:.0})", l, w, cap_l.max(cap_w))
                    } else {
                        String::new()
                    }
                )));
                x += l + r.gap;
            }
        }
        let natural = layer_trays.iter().map(|t| blocks[t.0].size[2]).fold(0.0, f64::max);
        // Exact fit: no lids, so the box lid holds the insert. The top layer rises to one
        // print-to-component tolerance under it (in whole layers).
        let fill = ((r.avail_h.mm - r.tray_box - z) / r.layer_h + 1e-6).floor() * r.layer_h;
        let top = li + 1 == nl;
        let h = if top && fill > natural + EPS && !r.tight { round3(fill) } else { natural };
        let area: f64 = layer_trays.iter().map(|t| t.3[0] * t.3[1]).sum();
        layer_meta.push(Layer { z: round3(z), height: h, coverage: round3(area / (r.usable[0] * r.usable[1])) });
        for (bi, turned, pos, size, fp_why) in layer_trays {
            let b = &blocks[bi];
            let tw = b.draft.tweak.clone().unwrap_or_default();
            // Pad to the layer height when something rests on top, so the next layer sits flat.
            let (th, hwhy) = if top && h > b.size[2] + EPS {
                (h, format!("{}; raised to {h:.1} mm so the insert fills to {} mm under the box lid (exact fit, no lids)", b.height_why, r.tray_box))
            } else if h > b.size[2] + EPS {
                (h, format!("{}; padded to the {h:.1} mm layer so the layer above sits flat", b.height_why))
            } else {
                (b.size[2], b.height_why.clone())
            };
            let (l, w) = (size[0], size[1]);
            // Pockets, transposed if the tray is turned in the box (all pockets are symmetric).
            let (in_l, in_w) = if turned { (b.size[1], b.size[0]) } else { (b.size[0], b.size[1]) };
            let mut pockets: Vec<Pocket> = vec![];
            for (q, p, pturned) in &b.placed {
                let nd = &needs[q.need];
                let (ql, qw) = if *pturned { (q.w, q.l) } else { (q.l, q.w) };
                let (px, py, sl, sw) = if turned { (p[1], p[0], qw, ql) } else { (p[0], p[1], ql, qw) };
                let (cl, cw) = if (sl >= sw) == (nd.content[0] >= nd.content[1]) {
                    (nd.content[0], nd.content[1])
                } else {
                    (nd.content[1], nd.content[0])
                };
                // Top-aligned at its own depth, unless that leaves only a thin step: then to the floor.
                let full = th - r.floor.mm;
                let depth = if full - q.depth < 3.0 { full } else { q.depth };
                let (lw, ww) = if sl >= sw { (&nd.pocket[0], &nd.pocket[1]) } else { (&nd.pocket[1], &nd.pocket[0]) };
                let open_ends = nd.kind == "cards" && tw.open_ends.unwrap_or(false);
                pockets.push(Pocket {
                    id: format!("{}-{}", b.draft.id, pockets.len() + 1),
                    component: nd.component.clone(),
                    label: q.label.clone(),
                    shape: nd.shape,
                    pos: [round3(b.draft.wall + px), round3(b.draft.wall + py)],
                    size: [round3(sl), round3(sw)],
                    depth: round3(depth),
                    content: [cl, cw, round3(q.content_h)],
                    items: q.items,
                    finger_hole: nd.finger_hole && !open_ends && tw.finger_holes.unwrap_or(true) && sl.min(sw) >= 30.0,
                    open_ends,
                    floor_window: open_ends,
                    scoop: nd.scoop && tw.pockets.get(&nd.component).and_then(|p| p.scoop).or(tw.scoops).unwrap_or(true),
                    notch: tw.pockets.get(&nd.component).and_then(|p| p.notch),
                    why: PocketWhy {
                        length: lw.why.clone(),
                        width: ww.why.clone(),
                        depth: if (depth - q.depth).abs() > EPS {
                            format!("{} = {:.2} mm, taken to the floor ({:.2} mm) rather than leave a thin step", q.why_depth, q.depth, depth)
                        } else {
                            q.why_depth.clone()
                        },
                    },
                });
            }
            // Spare room from stretching.
            let spare = |pockets: &mut Vec<Pocket>, pos: [f64; 2], size: [f64; 2]| {
                let n = pockets.len() + 1;
                pockets.push(Pocket {
                    id: format!("{}-{n}", b.draft.id),
                    component: String::new(),
                    label: "Spare".into(),
                    shape: Shape::Rect,
                    pos: [round3(pos[0]), round3(pos[1])],
                    size: [round3(size[0]), round3(size[1])],
                    depth: round3(th - r.floor.mm),
                    content: [0.0, 0.0, 0.0],
                    items: 0,
                    finger_hole: false,
                    open_ends: false,
                    floor_window: false,
                    scoop: tw.scoops.unwrap_or(true),
                    notch: None,
                    why: PocketWhy {
                        length: "room left after stretching the tray to fill the box".into(),
                        width: "room left after stretching the tray to fill the box".into(),
                        depth: "tray height less the floor".into(),
                    },
                });
            };
            let (bw, bd) = (b.draft.wall, b.draft.divider);
            let mut in_l = in_l - 2.0 * bw;
            let mut in_w = in_w - 2.0 * bw;
            let mut ex = l - 2.0 * bw - in_l;
            let mut ey = w - 2.0 * bw - in_w;
            // Grab holes (a tweak), from the room stretching left along the tray's length.
            let mut grab = None;
            // The band along the length a spare must stay out of: (axis, from, to).
            let mut band: Option<(usize, f64, f64)> = None;
            if let Some(want) = tw.half_circle.as_deref().filter(|_| !b.draft.holders) {
                let a = if l >= w { 0 } else { 1 };
                let ((inn, extra), (in_o, extra_o)) = if a == 0 { ((&mut in_l, &mut ex), (&mut in_w, &mut ey)) } else { ((&mut in_w, &mut ey), (&mut in_l, &mut ex)) };
                match grab_pair(spec, &mut pockets, [l, w], a, bw, bd, *extra, want) {
                    Ok((hc, used, loose)) => {
                        *inn += used;
                        *extra -= used;
                        // The room across the tray would make a spare along a long edge, past
                        // the holes. Loose pieces take it instead; otherwise the spare stops
                        // short of the holes on either side.
                        let o = 1 - a;
                        if loose && *extra_o > EPS {
                            let far = bw + *in_o;
                            for p in pockets.iter_mut().filter(|p| (p.pos[o] + p.size[o] - far).abs() < 0.5) {
                                p.size[o] = round3(p.size[o] + *extra_o);
                            }
                            *in_o += *extra_o;
                            *extra_o = 0.0;
                        }
                        let (c, r) = (hc.centers[0][a], hc.diameter / 2.0);
                        band = Some((a, c - r - bd, c + r + bd));
                        grab = Some(hc);
                    }
                    Err(e) => warnings.push(e.subject(&b.draft.id)),
                }
            }
            // A bin narrower than its neighbours (wider is easier to grab) takes the room
            // beside it up to the next pocket or the wall, and is made shallower to hold the same.
            if !b.draft.holders {
                let far = [bw + in_l, bw + in_w];
                for i in 0..pockets.len() {
                    if !spec.component(&pockets[i].component).is_some_and(|c| matches!(c.kind, Kind::Bin { .. })) {
                        continue;
                    }
                    for ax in 0..2 {
                        let (p, o) = (&pockets[i], 1 - ax);
                        let end = p.pos[ax] + p.size[ax];
                        let beside = |q: &Pocket| q.pos[o] < p.pos[o] + p.size[o] - EPS && p.pos[o] < q.pos[o] + q.size[o] - EPS;
                        let to = pockets.iter().enumerate().filter(|(j, q)| *j != i && beside(q) && q.pos[ax] >= end - EPS).map(|(_, q)| q.pos[ax] - bd).fold(far[ax], f64::min);
                        // Nor into the grab holes' band.
                        let to = match band {
                            Some((a, lo, _)) if a == ax && end <= lo + EPS => to.min(lo),
                            _ => to,
                        };
                        let depth = ((p.depth * p.size[ax] / (to - p.pos[ax]) * 10.0 - 1e-6).ceil() / 10.0).max(0.0);
                        if to - end < 1.0 || depth < p.depth / 2.0 {
                            continue;
                        }
                        let p = &mut pockets[i];
                        let why = format!("widened from {:.1} to {:.1} mm, to the {}: wider is easier to grab", p.size[ax], to - p.pos[ax], if (to - far[ax]).abs() < EPS { "wall" } else { "next pocket" });
                        p.why.depth = format!("{} mm, made shallower from {} mm to hold the same once widened", depth, p.depth);
                        if (ax == 0) == (p.size[0] >= p.size[1]) { p.why.length = why } else { p.why.width = why }
                        p.size[ax] = round3(to - p.pos[ax]);
                        p.depth = round3(depth);
                    }
                }
            }
            // A frame spreads its stretch into the gaps between bays: finger slots.
            let spread = |pockets: &mut Vec<Pocket>, axis: usize, extra: f64| {
                let mut starts: Vec<f64> = pockets.iter().map(|p| p.pos[axis]).collect();
                starts.sort_by(f64::total_cmp);
                starts.dedup_by(|a, b| (*a - *b).abs() < 0.5);
                if starts.len() < 2 || extra <= EPS {
                    return;
                }
                let step = extra / (starts.len() - 1) as f64;
                for p in pockets.iter_mut() {
                    let k = starts.iter().position(|s| (p.pos[axis] - s).abs() < 0.5).unwrap_or(0);
                    p.pos[axis] = round3(p.pos[axis] + k as f64 * step);
                }
            };
            if b.draft.holders {
                spread(&mut pockets, 0, ex);
                spread(&mut pockets, 1, ey);
            }
            // A spare never crosses the grab holes: it's cut either side of their band.
            let spare = |pockets: &mut Vec<Pocket>, pos: [f64; 2], size: [f64; 2]| match band {
                Some((a, lo, hi)) if pos[a] < hi && pos[a] + size[a] > lo => {
                    let (mut p, mut s) = (pos, size);
                    s[a] = lo - pos[a];
                    if s[a] >= SPARE_MIN - bd {
                        spare(pockets, p, s);
                    }
                    (p[a], s[a]) = (hi, pos[a] + size[a] - hi);
                    if s[a] >= SPARE_MIN - bd {
                        spare(pockets, p, s);
                    }
                }
                _ => spare(pockets, pos, size),
            };
            // A pocket asked to (a pocket tweak) takes the room left over beside it instead of a spare.
            let take = |pockets: &mut Vec<Pocket>, ax: usize, end: f64, extra: f64| -> bool {
                let Some(p) = pockets.iter_mut().find(|p| {
                    (p.pos[ax] + p.size[ax] - end).abs() < 0.5 && tw.pockets.get(&p.component).and_then(|x| x.fill_spare).unwrap_or(false)
                }) else {
                    return false;
                };
                p.size[ax] = round3(p.size[ax] + extra);
                let why = format!("takes the {extra:.1} mm left over in the tray rather than an empty spare (a pocket tweak)");
                if (ax == 0) == (p.size[0] >= p.size[1]) { p.why.length = why } else { p.why.width = why }
                true
            };
            if ex >= SPARE_MIN && !b.draft.holders && !take(&mut pockets, 0, bw + in_l, ex) {
                spare(&mut pockets, [bw + in_l + bd, bw], [ex - bd, in_w]);
            }
            if ey >= SPARE_MIN && !b.draft.holders && !take(&mut pockets, 1, bw + in_w, ey) {
                let sl = if ex >= SPARE_MIN { l - 2.0 * bw } else { in_l };
                spare(&mut pockets, [bw, bw + in_w + bd], [sl, ey - bd]);
            }
            let bed_fit = (l <= r.bed_max[0] + EPS && w <= r.bed_max[1] + EPS) || (w <= r.bed_max[0] + EPS && l <= r.bed_max[1] + EPS);
            let color = tw.color.clone().unwrap_or_else(|| PALETTE[trays.len() % PALETTE.len()].to_string());
            let pos = [round3(r.tray_box + pos[0]), round3(r.tray_box + pos[1]), round3(z)];
            // A frame: each pocket becomes a bay through the floor, and its contents go in a
            // lift-out holder standing on the box floor inside it.
            let mut holder_trays = vec![];
            if b.draft.holders {
                let (hw, g) = (spec.rules.walls.min, HOLDER_GAP);
                let bays: Vec<([f64; 2], [f64; 2])> = pockets.iter().map(|q| (q.pos, q.size)).collect();
                for (k, q) in pockets.iter_mut().enumerate() {
                    let size = [q.size[0] - 2.0 * g, q.size[1] - 2.0 * g];
                    // A notch set on the bay is the holder's: slots through its long walls, and
                    // with a floor bite in place of its floor window.
                    let notch = q.notch.take();
                    let bite = notch.is_some_and(|n| n.floor > 0.0);
                    let lips: Vec<Lip> = holder_lip(&bays, k, [size[0], size[1], th], &spec.rules.grips, notch.is_some()).filter(|_| tw.grips.unwrap_or(false)).into_iter().collect();
                    let reach = lips.iter().map(|l| l.depth).sum::<f64>();
                    let (fl, fw) = (size[0].max(size[1]) + reach, size[0].min(size[1]) + reach);
                    let bed_fit = fl <= r.bed_max[0].max(r.bed_max[1]) + EPS && fw <= r.bed_max[0].min(r.bed_max[1]) + EPS;
                    let (ppos, psize) = ([hw, hw], [size[0] - 2.0 * hw, size[1] - 2.0 * hw]);
                    let hid = format!("{}-h{}", b.draft.id, k + 1);
                    holder_trays.push(Tray {
                        id: hid.clone(),
                        name: q.label.clone(),
                        auto_name: q.label.clone(),
                        request: None,
                        layer: li,
                        pos: [round3(pos[0] + q.pos[0] + g), round3(pos[1] + q.pos[1] + g), pos[2]],
                        size: [round3(size[0]), round3(size[1]), round3(th)],
                        wall: hw,
                        corner_radius: spec.rules.walls.corner_radius_for(hw),
                        floor: r.floor.mm,
                        color: PALETTE[(trays.len() + 1 + k) % PALETTE.len()].to_string(),
                        pockets: vec![Pocket {
                            id: format!("{hid}-1"),
                            component: q.component.clone(),
                            label: q.label.clone(),
                            shape: q.shape,
                            pos: [round3(ppos[0]), round3(ppos[1])],
                            size: [round3(psize[0]), round3(psize[1])],
                            depth: q.depth,
                            content: q.content,
                            items: q.items,
                            finger_hole: false,
                            open_ends: false,
                            floor_window: !bite,
                            scoop: false,
                            notch,
                            why: q.why.clone(),
                        }],
                        bed_fit,
                        frame: false,
                        holder_of: Some(b.draft.id.clone()),
                        why: TrayWhy {
                            height: format!("{} mm, the frame's height", round3(th)),
                            footprint: format!(
                                "its bay in {} less {g} mm a side, to lift out{}",
                                b.draft.id,
                                lips.first().map_or(String::new(), |l| format!(
                                    "; a {} mm 45° lip (style grips) reaching {} mm into the finger slot beside it, to pull it up by",
                                    l.size[1 - l.axis], l.depth
                                ))
                            ),
                            walls: format!(
                                "{hw} mm walls (style walls.min) all round, braced by the frame; {}",
                                match notch {
                                    Some(n) if n.floor > 0.0 => format!("{} mm card-draw slots down through both long walls, with a {} mm half-circle bite into the floor to push the deck up", n.width, round3(crate::geometry::bite_radius(&n))),
                                    Some(n) => format!("{} mm card-draw slots in both long walls, and a floor window to push the deck up", n.width),
                                    None => "a floor window to push the deck up".into(),
                                }
                            ),
                        },
                        lips,
                        half_circle: None,
                        labels: vec![],
                        kept_edit: None,
                    });
                    q.label = format!("Bay for {}", q.label);
                    q.component = String::new();
                    q.depth = round3(th);
                    q.content = [0.0; 3];
                    q.items = 0;
                    (q.finger_hole, q.open_ends, q.floor_window, q.scoop) = (false, false, false, false);
                    q.why.depth = "through the frame: the holder stands on the box floor".into();
                }
            }
            trays.push(Tray {
                id: b.draft.id.clone(),
                name: tw.name.clone().unwrap_or_else(|| b.draft.name.clone()),
                auto_name: b.draft.name.clone(),
                request: tw.request.clone(),
                layer: li,
                pos,
                size: [round3(l), round3(w), round3(th)],
                wall: b.draft.wall,
                corner_radius: spec.rules.walls.corner_radius_for(b.draft.wall),
                floor: r.floor.mm,
                color,
                pockets,
                bed_fit,
                frame: b.draft.holders,
                holder_of: None,
                why: TrayWhy { height: hwhy, footprint: fp_why, walls: b.draft.walls_why.clone() },
                lips: vec![],
                half_circle: grab,
                labels: vec![],
                kept_edit: None,
            });
            trays.extend(holder_trays);
        }
        z += h;
    }
    trays.sort_by(|a, b| a.layer.cmp(&b.layer).then(a.id.cmp(&b.id)));
    // Labels go on last, on the trays as they'll be built.
    for i in 0..trays.len() {
        let frame = trays[i].holder_of.as_deref().and_then(|f| tweak_for(f));
        trays[i].labels = crate::labels::for_tray(spec, &trays[i], tweak_for(&trays[i].id).as_ref(), frame.as_ref(), &mut warnings);
        trays[i].kept_edit = tweak_for(&trays[i].id).and_then(|w| w.edit);
    }

    // 6. Report.
    for t in &gb.layout.tweaks {
        if !trays.iter().any(|x| x.id == t.tray) {
            warnings.push(
                Reason::new("tweak_unused", format!("tweak for tray '{}' matches no tray in this layout", t.tray)).subject(&t.tray),
            );
        } else if strategy == "fewest-trays" && t.max_depth.is_some() {
            warnings.push(
                Reason::new("tweak_ignored", "max_depth tweaks don't apply to fewest-trays (its trays are numbered after packing)")
                    .subject(&t.tray),
            );
        }
    }
    for (li, l) in layer_meta.iter().enumerate().skip(1) {
        let below: Vec<&Tray> = trays.iter().filter(|t| t.layer == li - 1).collect();
        for t in trays.iter().filter(|t| t.layer == li) {
            let s = support(t, &below);
            if s < 0.9 {
                warnings.push(
                    Reason::new("unsupported", format!("tray {} on layer {} rests on trays for only {:.0}% of its floor", t.id, li + 1, s * 100.0))
                        .subject(&t.id),
                );
            }
        }
        let _ = l;
    }
    let stack = z;
    // As-is components with a measured size, stacked on the insert, centred, long side along the
    // box's long side.
    let mut on_top = vec![];
    let mut tz = stack;
    for c in spec.components.iter().filter(|c| matches!(c.kind, Kind::AsIs) && c.place.is_none() && c.piece.is_none() && c.box_in(spec) == Some(gb.id.as_str())) {
        let Some(d) = c.measure.as_ref().and_then(|m| m.piece.or(m.stack)) else { continue };
        let (a, b) = (d[0].max(d[1]), d[0].min(d[1]));
        let (l, w) = if r.usable[0] >= r.usable[1] { (a, b) } else { (b, a) };
        let at = |room: f64, v: f64| round3(r.tray_box + ((room - v) / 2.0).max(0.0));
        on_top.push(OnTop { component: c.id.clone(), label: c.name.clone(), pos: [at(r.usable[0], l), at(r.usable[1], w), round3(tz)], size: [l, w, d[2]] });
        tz += d[2];
    }
    if tz - stack > gb.size.top_stack + EPS {
        warnings.push(
            Reason::new(
                "top_stack",
                format!("what lies on top of the insert is {:.1} mm thick, more than the box's {} mm top stack: raise top_stack, or the lid may not close", tz - stack, gb.size.top_stack),
            )
            .subject(&gb.id),
        );
    }
    let clearance = r.avail_h.mm - stack;
    // The box lid is a component too: the stack keeps the print-to-component gap under it.
    let fits_lid = clearance >= r.tray_box - EPS;
    if !fits_lid {
        problems.push(Reason::new(
            "over_lid",
            format!(
                "the trays stack {stack:.1} mm; with {} mm under the lid that's {:.1} mm more than the {:.1} mm of room",
                r.tray_box,
                r.tray_box - clearance,
                r.avail_h.mm
            ),
        ));
    }
    for t in trays.iter().filter(|t| !t.bed_fit) {
        problems.push(Reason::new("bed_fit", format!("tray {} doesn't fit the bed", t.id)).subject(&t.id));
    }
    let st = &spec.rules;
    let style_value = |k: &str| -> String {
        let v = toml::Value::try_from(st).ok();
        let got = k.split('.').try_fold(v, |v, part| v.and_then(|v| v.get(part).cloned()).map(Some)).flatten();
        got.map(|v| v.to_string()).unwrap_or_default()
    };
    let unconfirmed = st
        .proposed
        .iter()
        .map(|k| format!("style {k} = {} mm (proposed)", style_value(k)))
        .chain(spec.sleeves.iter().filter(|s| s.source != crate::spec::Source::Measured).map(|s| format!("sleeve '{}' {} mm/card ({:?})", s.id, s.per_card_mm, s.source).to_lowercase()))
        .collect();
    let lid_lift = if lift > 0.0 {
        let why = format!("the insert needs {lift:.1} mm above the rim; the box allows up to {} mm of lid lift", gb.size.lid_lift);
        warnings.push(Reason::new("lid_lift", format!("the closed lid sits {lift:.1} mm high: {why}")).subject(&gb.id));
        dim(lift, why)
    } else {
        dim(0.0, if gb.size.lid_lift > 0.0 { "none needed: the insert fits under a closed lid" } else { "none: the box allows no lid lift" })
    };
    if level > 0 {
        warnings.push(Reason::new("walls_reduced", format!("needed {} to fit (style normal is {} mm walls, {} mm floor)", LEVELS[level], st.walls.normal, st.floor.normal)));
    }
    let stack_why = layer_meta.iter().map(|l| format!("{:.1}", l.height)).collect::<Vec<_>>().join(" + ");
    Layout {
        strategy: strategy.to_string(),
        title: title.into(),
        description: description.into(),
        ok: problems.is_empty(),
        report: FitReport {
            available_height: r.avail_h.clone(),
            lid_lift,
            stack_height: dim(stack, if stack_why.is_empty() { "no trays".into() } else { format!("layers {stack_why} mm") }),
            lid_clearance: dim(clearance, "available height − stack height"),
            fits_lid,
            bed_fit: trays.iter().all(|t| t.bed_fit),
            tray_count: trays.len(),
            layer_count: layer_meta.len(),
            unconfirmed,
            walls_level: LEVELS[level].into(),
            wall: r.wall.clone(),
            wall_max: r.wall_max.clone(),
            divider: r.divider.clone(),
            floor: r.floor.clone(),
        },
        trays,
        layers: layer_meta,
        problems,
        warnings,
        printed: None,
        on_top,
    }
}

/// A printed box: the smallest box (by outside volume) that holds its trays. The search packs
/// loose in a floor from the bed's down in 5 mm steps (both sides capped, so it is pushed to
/// more compact shapes), and at
/// every pocket depth the normal search would try, and keeps the packing that makes the smallest
/// box. That layout is then packed again in the box it found, which stretches trays to fill it.
fn generate_printed(spec: &Spec, gb: &GameBox, strategy: &str) -> Layout {
    // The search runs thousands of packings, and every state refresh asks for each strategy more
    // than once, so results are kept by everything they depend on.
    use std::collections::HashMap;
    use std::hash::{Hash, Hasher};
    static MEMO: std::sync::Mutex<Option<HashMap<u64, Layout>>> = std::sync::Mutex::new(None);
    let key = {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let parts: Vec<_> = spec.packed_in(&gb.id).collect();
        serde_json::to_string(&(gb, strategy, &parts, &spec.rules, &spec.printer, &spec.groups, &spec.sleeves)).unwrap().hash(&mut h);
        h.finish()
    };
    if let Some(l) = MEMO.lock().unwrap().as_ref().and_then(|m| m.get(&key)) {
        return l.clone();
    }
    let l = size_printed(spec, gb, strategy);
    let mut memo = MEMO.lock().unwrap();
    let m = memo.get_or_insert_with(HashMap::new);
    if m.len() >= 64 {
        m.clear();
    }
    m.insert(key, l.clone());
    l
}

fn size_printed(spec: &Spec, gb: &GameBox, strategy: &str) -> Layout {
    let pb = &spec.rules.printed_box;
    let p = &spec.printer;
    let tb = spec.rules.tolerance.print_to_print;
    let lid_t = 2.0 * pb.lid_groove;
    let bed = [p.bed[0].max(p.bed[1]) - 2.0 * p.bed_margin, p.bed[0].min(p.bed[1]) - 2.0 * p.bed_margin];
    let mut max = [bed[0] - 2.0 * pb.wall, bed[1] - 2.0 * pb.wall, p.max_z - pb.floor - lid_t];
    let mut limit = "the largest box the bed prints".to_string();
    if let Some(m) = gb.printed.as_ref().and_then(|x| x.max_inner) {
        for i in 0..3 {
            if m[i] > 0.0 {
                max[i] = max[i].min(m[i]);
            }
        }
        limit = format!("at most {:.0} × {:.0} × {:.0} mm inside", max[0], max[1], max[2]);
    }
    let sized = |inner: [f64; 3]| {
        let mut g = gb.clone();
        g.size = BoxSpec { inner, lid_depth: 0.0, lid_lift: 0.0, ..gb.size.clone() };
        g
    };
    let probe = sized(max);
    let r = rules(spec, &probe, 0, 0.0);
    // (outside volume, pocket depth cap, inside, the packing floor and room it was found with)
    let mut best: Option<(f64, f64, [f64; 3], [f64; 2], f64)> = None;
    let mut failed: Option<Layout> = None;
    let mut len = max[0];
    while len > 0.0 {
        let probe = sized([len, len.min(max[1]), max[2]]);
        let pr = rules(spec, &probe, 0, 0.0);
        let mut any = false;
        let mut cap = pr.avail_h.mm - pr.floor.mm;
        while cap >= 6.0 {
            let l = generate_capped(spec, &probe, strategy, cap, 0, 0.0, Mode::Sizing);
            if l.ok && !l.trays.is_empty() {
                any = true;
                // Tray positions already include the gap to the near walls.
                let ext = |i: usize| l.trays.iter().map(|t| t.pos[i] + t.size[i]).fold(0.0, f64::max);
                let inner = [round3(ext(0) + tb), round3(ext(1) + tb), round3(ext(2) + tb + gb.size.top_stack)];
                let vol = (inner[0] + 2.0 * pb.wall) * (inner[1] + 2.0 * pb.wall) * (inner[2] + pb.floor + lid_t);
                if best.as_ref().is_none_or(|b| vol < b.0 - EPS) {
                    best = Some((vol, cap, inner, pr.usable, pr.pack_room));
                }
            } else if failed.is_none() {
                failed = Some(l);
            }
            cap -= 2.0;
        }
        // Shorter only packs worse once nothing fits.
        if !any {
            break;
        }
        len -= 5.0;
    }
    let Some((_, cap, inner, pack, room)) = best else {
        let mut l = failed.unwrap_or_else(|| generate_capped(spec, &probe, strategy, r.avail_h.mm - r.floor.mm, 0, 0.0, Mode::Sizing));
        if !l.trays.is_empty() || !l.problems.is_empty() {
            l.problems.push(Reason::new("printed_too_big", format!("what goes in doesn't fit {limit}")).subject(&gb.id));
            l.ok = false;
        }
        return l;
    };
    let real = sized(inner);
    let mut l = generate_capped(spec, &real, strategy, cap, 0, 0.0, Mode::Sized { pack, room });
    let (w, d, g) = (pb.wall, pb.lid_groove, pb.slide_gap);
    let outer = [round3(inner[0] + 2.0 * w), round3(inner[1] + 2.0 * w), round3(inner[2] + pb.floor + lid_t)];
    let bed_fit = (outer[0] <= bed[0] + EPS && outer[1] <= bed[1] + EPS) && outer[2] <= p.max_z + EPS;
    if !bed_fit {
        l.problems.push(Reason::new("bed_fit", format!("the printed box ({:.1} × {:.1} × {:.1} mm) doesn't fit the bed", outer[0], outer[1], outer[2])).subject(&gb.id));
        l.ok = false;
    }
    let top = if gb.size.top_stack > 0.0 { format!(" + {} mm top stack", gb.size.top_stack) } else { String::new() };
    l.printed = Some(PrintedShell {
        inner,
        outer,
        wall: w,
        floor: pb.floor,
        groove: d,
        gap: g,
        detent: pb.detent,
        lid: [round3(inner[0] + w + d - g), round3(inner[1] + 2.0 * d - 2.0 * g), round3(lid_t - g)],
        bed_fit,
        why: format!(
            "the smallest box that holds the trays ({:.0} mm deep pockets){top}, {tb} mm print-to-print gaps; walls {w} mm, floor {} mm, a {:.1} mm slide-on lid (style printed_box{}); {limit}",
            cap,
            pb.floor,
            lid_t - g,
            spec.rules.mark("printed_box.wall")
        ),
    });
    l
}

/// Share of a tray's footprint that rests on trays below (sampled on a 4 mm grid).
fn support(t: &Tray, below: &[&Tray]) -> f64 {
    let (mut n, mut ok) = (0u32, 0u32);
    let step = 4.0;
    let mut x = t.pos[0] + step / 2.0;
    while x < t.pos[0] + t.size[0] {
        let mut y = t.pos[1] + step / 2.0;
        while y < t.pos[1] + t.size[1] {
            n += 1;
            // Gaps between trays below (≤ 1.5 mm) still count as support.
            if below.iter().any(|b| x >= b.pos[0] - 1.5 && x <= b.pos[0] + b.size[0] + 1.5 && y >= b.pos[1] - 1.5 && y <= b.pos[1] + b.size[1] + 1.5) {
                ok += 1;
            }
            y += step;
        }
        x += step;
    }
    if n == 0 { 1.0 } else { ok as f64 / n as f64 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample;

    #[test]
    fn sample_fits_every_strategy_and_is_deterministic() {
        let s = sample::spec();
        for (id, _, _) in STRATEGIES {
            let a = generate(&s, &s.boxes()[0], id);
            assert!(a.ok, "{id}: {:?}", a.problems);
            assert!(a.report.lid_clearance.mm >= 0.0);
            assert!(a.trays.iter().all(|t| t.bed_fit));
            let b = generate(&s, &s.boxes()[0], id);
            assert_eq!(serde_json::to_string(&a).unwrap(), serde_json::to_string(&b).unwrap());
        }
    }

    #[test]
    fn every_component_is_placed() {
        let s = sample::spec();
        let l = generate(&s, &s.boxes()[0], "by-group");
        for c in s.components.iter().filter(|c| c.packs()) {
            assert!(l.trays.iter().any(|t| t.pockets.iter().any(|p| p.component == c.id)), "{} not placed", c.id);
        }
    }

    #[test]
    fn trays_stay_inside_the_box_and_apart() {
        let s = sample::spec();
        for (id, _, _) in STRATEGIES {
            let l = generate(&s, &s.boxes()[0], id);
            for t in &l.trays {
                assert!(t.pos[0] + t.size[0] <= s.boxes()[0].size.inner[0] + EPS && t.pos[1] + t.size[1] <= s.boxes()[0].size.inner[1] + EPS, "{id} {}", t.id);
                for u in l.trays.iter().filter(|u| u.layer == t.layer && u.id != t.id) {
                    let apart = t.pos[0] + t.size[0] <= u.pos[0] + EPS
                        || u.pos[0] + u.size[0] <= t.pos[0] + EPS
                        || t.pos[1] + t.size[1] <= u.pos[1] + EPS
                        || u.pos[1] + u.size[1] <= t.pos[1] + EPS;
                    assert!(apart, "{id}: {} overlaps {}", t.id, u.id);
                }
            }
        }
    }

    #[test]
    fn too_tall_for_the_box_is_a_reason_not_a_panic() {
        let mut s = sample::spec();
        s.pick_box_mut(None).unwrap().box_kind_mut().inner[2] = 30.0;
        let l = generate(&s, &s.boxes()[0], "by-kind");
        assert!(!l.ok);
        assert!(l.problems.iter().any(|p| p.code == "too_tall" || p.code == "over_lid"));
    }

    #[test]
    fn max_depth_tweak_splits_stacks() {
        let mut s = sample::spec();
        let before = generate(&s, &s.boxes()[0], "by-kind").trays.iter().find(|t| t.id == "t-tokens").unwrap().pockets.len();
        s.pick_box_mut(None).unwrap().box_kind_mut().layout.tweaks.push(Tweak { tray: "t-tokens".into(), max_depth: Some(8.0), ..Default::default() });
        let l = generate(&s, &s.boxes()[0], "by-kind");
        let t = l.trays.iter().find(|t| t.id.starts_with("t-tokens")).unwrap();
        assert!(t.pockets.iter().filter(|p| p.items > 0).all(|p| p.content[2] + 1.0 <= 8.0 + EPS));
        assert!(l.trays.iter().filter(|t| t.id.starts_with("t-tokens")).map(|t| t.pockets.len()).sum::<usize>() > before);
    }

    #[test]
    fn style_tolerances_and_walls_are_applied() {
        let mut s = sample::spec();
        let l = generate(&s, &s.boxes()[0], "by-kind");
        let tol = s.rules.tolerance.print_to_component;
        // 0.8 mm per side around a 66 × 91 card.
        let card = l.trays.iter().flat_map(|t| &t.pockets).find(|p| p.component == "event-deck").unwrap();
        let mut got = card.size;
        got.sort_by(f64::total_cmp);
        assert!((got[0] - (66.0 + 2.0 * tol)).abs() < 1e-6 && (got[1] - (91.0 + 2.0 * tol)).abs() < 1e-6, "{got:?}");
        // The insert fills to one tolerance under the lid.
        assert!((l.report.lid_clearance.mm - tol).abs() < s.printer.layer_height + 1e-6, "{}", l.report.lid_clearance.mm);
        // Trays sit at least the print-to-print gap apart and the tolerance off the box walls.
        for t in &l.trays {
            assert!(t.pos[0] >= tol - 1e-6 && t.pos[1] >= tol - 1e-6);
            assert!(t.pos[0] + t.size[0] <= s.boxes()[0].size.inner[0] - tol + 1e-6);
        }
        // A heavy card stack gets the reinforced wall.
        s.components.push(crate::spec::Component {
            id: "brick".into(),
            name: "Brick deck".into(),
            group: None,
            source: crate::spec::Source::Sample,
            clearance: None,
            photos: vec![],
            faces: Default::default(),
            measure: None,
            notes: None,
            label: None,
            goes_with: vec![],
            box_: None,
            piece: None,
            pockets: vec![],
            place: None,
            kind: Kind::Cards { size: [66.0, 91.0], count: None, stack: Some(45.0), sleeve: None },
        });
        let l = generate(&s, &s.boxes()[0], "by-kind");
        let t = l.trays.iter().find(|t| t.pockets.iter().any(|p| p.component == "brick")).unwrap();
        if t.pockets.iter().any(|p| p.component == "brick" && p.content[2] >= s.rules.heavy_cards_mm) {
            assert_eq!(t.wall, s.rules.walls.max);
        }
    }

    #[test]
    fn per_pocket_card_holders_lift_out_one_deck_each() {
        let mut s = sample::spec();
        let tweak = Tweak { tray: "t-cards".into(), per_pocket: Some(true), open_ends: Some(true), ..Default::default() };
        s.pick_box_mut(None).unwrap().box_kind_mut().layout.tweaks.push(tweak);
        let l = generate(&s, &s.boxes()[0], "by-kind");
        assert!(l.ok, "{:?}", l.problems);
        let cards: Vec<_> = l.trays.iter().filter(|t| t.pockets.iter().any(|p| p.items > 0 && p.open_ends)).collect();
        assert!(cards.len() > 1);
        for t in cards {
            let held: Vec<_> = t.pockets.iter().filter(|p| p.items > 0).collect();
            assert_eq!(held.len(), 1, "{}: one deck per tray", t.id);
            assert!(held[0].open_ends && !held[0].finger_hole, "{}: a card holder, no floor hole", t.id);
            assert_eq!(t.name, held[0].label);
        }
    }

    #[test]
    fn a_frame_of_holders_has_a_holder_in_each_bay() {
        let mut s = sample::spec();
        let tweak = Tweak { tray: "t-cards".into(), holders: Some(true), ..Default::default() };
        s.pick_box_mut(None).unwrap().box_kind_mut().layout.tweaks.push(tweak);
        let l = generate(&s, &s.boxes()[0], "by-kind");
        assert!(l.ok, "{:?}", l.problems);
        let frames: Vec<_> = l.trays.iter().filter(|t| t.frame).collect();
        assert!(!frames.is_empty());
        for f in frames {
            assert!(f.pockets.iter().all(|p| p.items == 0 && (p.depth - f.size[2]).abs() < 1e-6), "{}: bays go through", f.id);
            let holders: Vec<_> = l.trays.iter().filter(|t| t.holder_of.as_deref() == Some(&f.id)).collect();
            assert_eq!(holders.len(), f.pockets.len());
            for (h, bay) in holders.iter().zip(&f.pockets) {
                let p = &h.pockets[0];
                assert!(p.items > 0 && !p.open_ends && p.floor_window && !p.finger_hole);
                assert!(p.pos.iter().all(|&x| (x - h.wall).abs() < 1e-6), "{}: four walls", h.id);
                for i in 0..2 {
                    let at = f.pos[i] + bay.pos[i];
                    assert!(h.pos[i] >= at - 1e-6 && h.pos[i] + h.size[i] <= at + bay.size[i] + 1e-6, "{} inside its bay", h.id);
                }
            }
        }
    }

    #[test]
    fn holder_grips_reach_into_the_slot_and_miss_each_other() {
        let mut s = sample::spec();
        let tweak = Tweak { tray: "t-cards".into(), holders: Some(true), grips: Some(true), ..Default::default() };
        s.pick_box_mut(None).unwrap().box_kind_mut().layout.tweaks.push(tweak);
        let l = generate(&s, &s.boxes()[0], "by-kind");
        assert!(l.ok, "{:?}", l.problems);
        let holders: Vec<_> = l.trays.iter().filter(|t| t.holder_of.is_some()).collect();
        // Each lip in box coordinates, as a footprint rectangle.
        let rect = |t: &Tray, p: [f64; 2], sz: [f64; 2]| [t.pos[0] + p[0], t.pos[1] + p[1], t.pos[0] + p[0] + sz[0], t.pos[1] + p[1] + sz[1]];
        let overlaps = |a: [f64; 4], b: [f64; 4]| a[0] < b[2] - 1e-6 && b[0] < a[2] - 1e-6 && a[1] < b[3] - 1e-6 && b[1] < a[3] - 1e-6;
        let lips: Vec<_> = holders.iter().flat_map(|t| t.lips.iter().map(move |lp| (t, rect(t, lp.pos, lp.size)))).collect();
        assert!(!lips.is_empty(), "the sample's card frame has a finger slot");
        for (t, lr) in &lips {
            let lp = &t.lips[0];
            assert!(t.lips.len() == 1 && lp.depth <= s.rules.grips.lip_depth + 1e-9, "{}: one lip, no deeper than the style's", t.id);
            assert!(lp.pos[lp.axis] >= t.size[lp.axis] - 1e-6 || lp.pos[lp.axis] + lp.size[lp.axis] <= 1e-6, "{}: outside its footprint", t.id);
            for h in &holders {
                if h.id != t.id {
                    assert!(!overlaps(*lr, rect(h, [0.0, 0.0], [h.size[0], h.size[1]])), "{}'s lip runs into {}", t.id, h.id);
                }
            }
            for (u, other) in &lips {
                if u.id != t.id {
                    assert!(!overlaps(*lr, *other), "{} and {} lips collide", t.id, u.id);
                }
            }
            let (lo, hi) = t.print_bounds();
            assert!(hi[lp.axis] - lo[lp.axis] > t.size[lp.axis], "print bounds take in the lip");
        }
    }

    #[test]
    fn grab_holes_come_in_pairs_clear_of_every_pocket() {
        let mut s = sample::spec();
        let base = generate(&s, &s.boxes()[0], "by-group");
        // A tray of its own group (tweaks name the group's tray) with a row of pockets.
        let row = |t: &&Tray| {
            let o = if t.size[0] >= t.size[1] { 1 } else { 0 };
            let held: Vec<_> = t.pockets.iter().filter(|p| p.items > 0).collect();
            held.len() > 1 && held.iter().all(|p| (p.pos[o] - held[0].pos[o]).abs() < 0.5)
        };
        let t = base.trays.iter().filter(|t| t.holder_of.is_none() && !t.frame && !t.id.rsplit('-').next().unwrap().chars().all(|c| c.is_ascii_digit())).find(row);
        let t = t.unwrap_or_else(|| panic!("{:?}", base.trays.iter().map(|t| (&t.id, t.pockets.len())).collect::<Vec<_>>())).clone();
        s.pick_box_mut(None).unwrap().box_kind_mut().layout.tweaks.push(Tweak { tray: t.id.clone(), half_circle: Some("middle".into()), ..Default::default() });
        let l = generate(&s, &s.boxes()[0], "by-group");
        let t = l.trays.iter().find(|x| x.id == t.id).unwrap();
        let Some(hc) = t.half_circle.as_ref() else { panic!("{}: no grab holes: {:?} {:?}", t.id, t.pockets.iter().map(|p| (p.pos, p.size)).collect::<Vec<_>>(), l.warnings) };
        let a = if t.size[0] >= t.size[1] { 0 } else { 1 };
        let o = 1 - a;
        let r = hc.diameter / 2.0;
        assert!((hc.diameter - s.rules.grips.half_circle).abs() < 1e-9);
        assert!((hc.centers[0][a] - hc.centers[1][a]).abs() < 1e-9 && hc.centers[0][o].abs() < 1e-9 && (hc.centers[1][o] - t.size[o]).abs() < 1e-9, "opposite sides: {hc:?}");
        let c = hc.centers[0][a];
        for p in &t.pockets {
            assert!(p.pos[a] >= c + r - 1e-6 || p.pos[a] + p.size[a] <= c - r + 1e-6, "{} runs into the grab holes at {c}", p.id);
        }
        assert!(t.pockets.iter().all(|p| p.pos[a] + p.size[a] <= t.size[a] - t.wall + 1e-6), "pockets still inside the tray");
    }

    #[test]
    fn labels_sit_on_solid_faces_clear_of_every_cut() {
        let mut s = sample::spec();
        let base = generate(&s, &s.boxes()[0], "by-group");
        let tweaks = &mut s.pick_box_mut(None).unwrap().box_kind_mut().layout.tweaks;
        for t in base.trays.iter().filter(|t| t.holder_of.is_none()) {
            tweaks.push(Tweak { tray: t.id.clone(), label: Some("Ab".into()), pocket_labels: Some(true), ..Default::default() });
        }
        let l = generate(&s, &s.boxes()[0], "by-group");
        let st = &s.rules.labels;
        let placed: usize = l.trays.iter().map(|t| t.labels.len()).sum();
        assert!(placed > 0, "{:?}", l.warnings);
        let overlaps = |p: [f64; 2], z: [f64; 2], a: [f64; 2], b: [f64; 2]| p[0] < a[0] + b[0] - 1e-6 && a[0] < p[0] + z[0] - 1e-6 && p[1] < a[1] + b[1] - 1e-6 && a[1] < p[1] + z[1] - 1e-6;
        for t in &l.trays {
            for lb in &t.labels {
                assert!(lb.height >= st.min_height - 1e-9 && lb.height <= st.height + 1e-9, "{}: {}", t.id, lb.height);
                assert!(lb.pos[0] >= -1e-6 && lb.pos[1] >= -1e-6 && lb.pos[0] + lb.size[0] <= t.size[0] + 1e-6 && lb.pos[1] + lb.size[1] <= t.size[1] + 1e-6, "{}: off the tray", t.id);
                if lb.on == "top" {
                    // On the top: clear of every pocket.
                    for q in &t.pockets {
                        assert!(!overlaps(lb.pos, lb.size, q.pos, q.size), "{}: '{}' runs into {}", t.id, lb.text, q.id);
                    }
                    assert!((lb.z - t.size[2]).abs() < 1e-6);
                } else {
                    // On a pocket floor: inside the pocket, clear of its finger hole and window.
                    let q = t.pockets.iter().find(|q| q.id == lb.on).unwrap();
                    assert!(lb.pos[0] >= q.pos[0] && lb.pos[1] >= q.pos[1] && lb.pos[0] + lb.size[0] <= q.pos[0] + q.size[0] && lb.pos[1] + lb.size[1] <= q.pos[1] + q.size[1], "{}: '{}' outside {}", t.id, lb.text, q.id);
                    if let Some(d) = q.finger_hole_diameter() {
                        let c = [q.pos[0] + q.size[0] / 2.0 - d / 2.0, q.pos[1] + q.size[1] / 2.0 - d / 2.0];
                        assert!(!overlaps(lb.pos, lb.size, c, [d, d]), "{}: over the finger hole", t.id);
                    }
                    assert!((lb.z - (t.size[2] - q.depth)).abs() < 1e-6 && lb.z > st.depth);
                }
            }
        }
        // The same spec gives the same labels.
        let again = generate(&s, &s.boxes()[0], "by-group");
        assert_eq!(serde_json::to_string(&l.trays).unwrap(), serde_json::to_string(&again.trays).unwrap());
    }

    #[test]
    fn below_the_style_minimums_is_refused() {
        let mut s = sample::spec();
        s.components[0].clearance = Some(0.5);
        assert_eq!(s.validate().unwrap_err().code, "below_tolerance");
        let mut s = sample::spec();
        s.pick_box_mut(None).unwrap().box_kind_mut().layout.tweaks.push(Tweak { tray: "t-shared".into(), walls: Some(1.0), ..Default::default() });
        assert_eq!(s.validate().unwrap_err().code, "out_of_style");
    }

    #[test]
    fn each_box_packs_only_its_own_components() {
        let mut s = sample::spec();
        let mut second = s.component("main").unwrap().clone();
        second.id = "expansion".into();
        s.components.insert(1, second);
        s.components.iter_mut().find(|c| c.id == "dice").unwrap().box_ = Some("expansion".into());
        s.components.iter_mut().find(|c| c.id == "score-pad").unwrap().kind = Kind::AsIs;
        s.components.iter_mut().find(|c| c.id == "goods").unwrap().kind = Kind::Undecided;
        s.validate().unwrap();
        let has = |l: &Layout, id: &str| l.trays.iter().any(|t| t.pockets.iter().any(|p| p.component == id));
        let main = chosen(&s, &s.boxes()[0]);
        let exp = chosen(&s, &s.boxes()[1]);
        assert!(has(&main, "coins") && !has(&main, "dice") && !has(&main, "score-pad") && !has(&main, "goods"));
        assert!(has(&exp, "dice") && !has(&exp, "coins"));
    }

    #[test]
    fn a_single_box_spec_loads_into_boxes() {
        let text = r#"
            [game]
            name = "Old"
            [box]
            inner = [200.0, 200.0, 60.0]
            [printer]
            name = "x"
            nozzle = 0.4
            bed = [256.0, 256.0]
            max_z = 256.0
            bed_margin = 3.0
            layer_height = 0.2
            line_width = 0.42
            [layout]
            strategy = "by-kind"
        "#;
        let dir = std::env::temp_dir().join(format!("bid-legacy-{}", crate::now_ms()));
        std::fs::create_dir_all(dir.join("old")).unwrap();
        std::fs::write(dir.join("old").join("spec.toml"), text).unwrap();
        let s = crate::spec::load(&dir, "old").unwrap();
        assert_eq!(s.boxes().len(), 1);
        assert_eq!(s.boxes()[0].layout.strategy, "by-kind");
        assert!(s.components[0].is_box());
        crate::spec::save(&dir, "old", &s).unwrap();
        let again = std::fs::read_to_string(dir.join("old").join("spec.toml")).unwrap();
        assert!(again.contains("kind = \"box\"") && !again.contains("[[boxes]]") && !again.contains("\n[box]") && !again.contains("\n[layout]"), "{again}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn lid_lift_is_used_only_as_needed() {
        let mut s = sample::spec();
        // Too short to close: without lift it doesn't fit.
        s.pick_box_mut(None).unwrap().box_kind_mut().inner[2] = 52.0;
        let l = generate(&s, &s.boxes()[0], "by-kind");
        assert!(!l.ok);
        s.pick_box_mut(None).unwrap().box_kind_mut().lid_lift = 15.0;
        let l = generate(&s, &s.boxes()[0], "by-kind");
        assert!(l.ok, "{:?}", l.problems);
        let used = l.report.lid_lift.mm;
        assert!(used > 0.0 && used < 15.0, "{used}");
        // One layer less doesn't fit: the lift is the smallest that does.
        let mut tight = s.clone();
        tight.pick_box_mut(None).unwrap().box_kind_mut().lid_lift = used - s.printer.layer_height;
        assert!(!generate(&tight, &tight.boxes()[0], "by-kind").ok);
        // A box that fits closed uses none.
        s.pick_box_mut(None).unwrap().box_kind_mut().inner[2] = 70.0;
        assert_eq!(generate(&s, &s.boxes()[0], "by-kind").report.lid_lift.mm, 0.0);
    }

    /// The sample's box made printable, holding only what fits a small box: the market cards,
    /// the goods and the dice.
    fn printed_sample() -> Spec {
        let mut s = sample::spec();
        let b = s.pick_box_mut(None).unwrap().box_kind_mut();
        b.printed = Some(crate::spec::PrintedBox::default());
        (b.inner, b.top_stack) = ([0.0; 3], 0.0);
        s.components.retain(|c| c.is_box() || ["market-deck", "goods", "dice"].contains(&c.id.as_str()));
        s
    }

    #[test]
    fn a_printed_box_is_sized_to_its_trays() {
        let s = printed_sample();
        s.validate().unwrap();
        let gb = &s.boxes()[0];
        for (id, _, _) in STRATEGIES {
            let l = generate(&s, gb, id);
            assert!(l.ok, "{id}: {:?}", l.problems);
            let p = l.printed.as_ref().expect("a printed shell");
            let tb = s.rules.tolerance.print_to_print;
            // Every tray inside, and the box no bigger than the trays need on any side.
            for i in 0..3 {
                let ext = l.trays.iter().map(|t| t.pos[i] + t.size[i]).fold(0.0, f64::max);
                assert!(ext + tb <= p.inner[i] + 1e-6, "{id} axis {i}: {ext} in {:?}", p.inner);
                assert!(ext + tb >= p.inner[i] - 1e-6, "{id} axis {i}: {ext} leaves room in {:?}", p.inner);
            }
            assert!(l.trays.iter().all(|t| t.pos[0] >= tb - 1e-6 && t.pos[1] >= tb - 1e-6));
            assert!(p.bed_fit && p.outer[0] < 160.0, "{id}: {:?}", p.outer);
            assert!((p.outer[2] - (p.inner[2] + p.floor + 2.0 * p.groove)).abs() < 1e-6);
            assert!(l.report.fits_lid);
            // Deterministic.
            assert_eq!(serde_json::to_string(&l).unwrap(), serde_json::to_string(&generate(&s, gb, id)).unwrap());
        }
    }

    #[test]
    fn a_printed_box_keeps_to_its_limit() {
        let mut s = printed_sample();
        let free = generate(&s, &s.boxes()[0], "by-kind").printed.unwrap().inner;
        s.pick_box_mut(None).unwrap().box_kind_mut().printed = Some(crate::spec::PrintedBox { max_inner: Some([0.0, 0.0, 40.0]) });
        let l = generate(&s, &s.boxes()[0], "by-kind");
        assert!(l.ok, "{:?}", l.problems);
        let p = l.printed.unwrap().inner;
        assert!(p[2] <= 40.0 + 1e-6, "{free:?} → {p:?}");
        s.pick_box_mut(None).unwrap().box_kind_mut().printed = Some(crate::spec::PrintedBox { max_inner: Some([40.0, 40.0, 0.0]) });
        let l = generate(&s, &s.boxes()[0], "by-kind");
        assert!(!l.ok && l.problems.iter().any(|p| p.code == "printed_too_big"), "{:?}", l.problems);
    }

    #[test]
    fn printed_sizing_is_quick() {
        // Best of three, each a full search (size_printed, past the memo): one run alone takes
        // ~250 ms, but the upkeep job runs this at 06:10 beside other scheduled work, where a
        // single run has taken 1.6 s and 4.6 s and flagged a release as broken that wasn't.
        let s = printed_sample();
        let gb = &s.boxes()[0];
        let mut ms = u128::MAX;
        let mut all = Vec::new();
        for _ in 0..3 {
            let t = std::time::Instant::now();
            all = STRATEGIES.iter().map(|(st, _, _)| size_printed(&s, gb, st)).collect::<Vec<_>>();
            ms = ms.min(t.elapsed().as_millis());
        }
        eprintln!("{ms} ms");
        for l in &all {
            eprintln!("{}: {:?} outside, {} trays", l.strategy, l.printed.as_ref().map(|p| p.outer), l.trays.len());
        }
        assert!(ms < 1500, "{ms} ms for every strategy");
    }

    #[test]
    fn an_empty_printed_box_has_no_shell() {
        let mut s = printed_sample();
        s.components.retain(|c| c.is_box());
        let l = generate(&s, &s.boxes()[0], "by-group");
        assert!(l.trays.is_empty() && l.printed.is_none() && l.problems.is_empty());
    }
}
