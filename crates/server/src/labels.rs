//! Stencil labels: where a tray's text goes and how big, worked out from the tray as packed.
//!
//! A label is cut `labels.depth` into a flat face that prints facing up: the tray's top, or a
//! pocket's floor. Inlaid, a separate part fills the cut flush, in its own filament (as on
//! printed trays that work: 0.6 mm deep, ~8 mm capitals, the tray recessed under each letter). Cut, it
//! stays open. Either way the face's top layers carry the text, so nothing needs supports.
//!
//! The packer's part: find the largest clear rectangle on the face (clear of edges, pockets,
//! grab holes, finger holes, floor windows and scoop fillets by `labels.margin`), lay the text
//! along or across it, and size it up to `labels.height`. Under `labels.min_height` the label
//! is left off with a warning, never squeezed.

use serde::Serialize;

use crate::Reason;
use crate::layout::{Pocket, Shape, Tray};
use crate::spec::{LabelMode, Spec, Tweak};

/// Advance widths of Liberation Sans, SolidCraft's built-in `sketch.text` font, per mm of
/// capital height, for ASCII 32 (space) to 126. Measured from SolidCraft v0.26.0's own output
/// (`H<c>H` less `HH`); it applies no kerning, so a line's width is the sum.
const ADVANCE: [f64; 95] = [
    0.4038, 0.4038, 0.5160, 0.8084, 0.8084, 1.2924, 0.9695, 0.2775, 0.4840, 0.4840,
    0.5656, 0.8488, 0.4038, 0.4840, 0.4038, 0.4038, 0.8084, 0.8084, 0.8084, 0.8084,
    0.8084, 0.8084, 0.8084, 0.8084, 0.8084, 0.8084, 0.4038, 0.4038, 0.8488, 0.8488,
    0.8488, 0.8084, 1.4755, 0.9695, 0.9695, 1.0497, 1.0497, 0.9695, 0.8879, 1.1306,
    1.0497, 0.4038, 0.7268, 0.9695, 0.8084, 1.2108, 1.0497, 1.1306, 0.9695, 1.1306,
    1.0497, 0.9695, 0.8879, 1.0497, 0.9695, 1.3719, 0.9695, 0.9695, 0.8879, 0.4038,
    0.4038, 0.4038, 0.6820, 0.8084, 0.4840, 0.8084, 0.8084, 0.7268, 0.8084, 0.8084,
    0.4038, 0.8084, 0.8084, 0.3229, 0.3229, 0.7268, 0.3229, 1.2108, 0.8084, 0.8084,
    0.8084, 0.8084, 0.4840, 0.7268, 0.4038, 0.8084, 0.7268, 1.0497, 0.7268, 0.7268,
    0.7268, 0.4855, 0.3776, 0.4855, 0.8488,
];
/// Any other character: as wide as an O.
const OTHER: f64 = 1.1306;
/// The ink's reach above the baseline (ascenders, brackets) and below it, per mm of capitals.
const ASCENT: f64 = 1.08;
const DESCENT: f64 = 0.31;
const DESCENDERS: &str = "gjpqy,;()[]{}|_@$Q";
const EPS: f64 = 1e-6;

#[derive(Serialize, Clone, Debug)]
pub struct Label {
    pub text: String,
    /// "top", or the id of the pocket whose floor it is on.
    pub on: String,
    pub mode: LabelMode,
    /// Height of the face it's cut into, tray-local, and how deep it's cut.
    pub z: f64,
    pub depth: f64,
    /// Capital height, mm.
    pub height: f64,
    /// 0: reads along x; 90: along y, its tops toward −x.
    pub angle: u32,
    /// The baseline's middle, tray-local: `sketch.text`'s `at` with `align: center`.
    pub at: [f64; 2],
    /// The box the ink stays inside, tray-local min corner and size.
    pub pos: [f64; 2],
    pub size: [f64; 2],
    pub why: String,
}

/// A line's width per mm of capital height.
fn width(text: &str) -> f64 {
    text.chars().map(|c| if (' '..='~').contains(&c) { ADVANCE[c as usize - 32] } else { OTHER }).sum()
}

/// How far the ink reaches below the baseline, per mm of capitals.
fn drop(text: &str) -> f64 {
    if text.chars().any(|c| DESCENDERS.contains(c) || !c.is_ascii()) { DESCENT } else { 0.02 }
}

fn round1_down(v: f64) -> f64 {
    ((v + EPS) * 10.0).floor() / 10.0
}

/// `[x0, y0, x1, y1]`.
type Rect = [f64; 4];

fn overlaps(a: Rect, b: Rect) -> bool {
    a[0] < b[2] - EPS && b[0] < a[2] - EPS && a[1] < b[3] - EPS && b[1] < a[3] - EPS
}

/// The best spot for `text` in `region` clear of `obstacles`: (capital height, angle, rect).
/// Every rectangle with sides on the region's and obstacles' edges is tried; the one giving the
/// tallest text wins, reading along x when that's as tall, then the roomiest.
fn best_spot(region: Rect, obstacles: &[Rect], text: &str, max_h: f64) -> Option<(f64, u32, Rect)> {
    let (w, t) = (width(text), ASCENT + drop(text));
    let mut xs = vec![region[0], region[2]];
    let mut ys = vec![region[1], region[3]];
    for o in obstacles {
        xs.extend([o[0], o[2]].into_iter().filter(|&x| x > region[0] && x < region[2]));
        ys.extend([o[1], o[3]].into_iter().filter(|&y| y > region[1] && y < region[3]));
    }
    for v in [&mut xs, &mut ys] {
        v.sort_by(f64::total_cmp);
        v.dedup_by(|a, b| (*a - *b).abs() < EPS);
    }
    let mut best: Option<(f64, u32, Rect)> = None;
    for (i, &x0) in xs.iter().enumerate() {
        for &x1 in &xs[i + 1..] {
            for (j, &y0) in ys.iter().enumerate() {
                for &y1 in &ys[j + 1..] {
                    let r = [x0, y0, x1, y1];
                    if obstacles.iter().any(|o| overlaps(r, *o)) {
                        continue;
                    }
                    let (lx, ly) = (x1 - x0, y1 - y0);
                    for (angle, h) in [(0, (lx / w).min(ly / t)), (90, (ly / w).min(lx / t))] {
                        let h = round1_down(h.min(max_h));
                        let area = lx * ly;
                        let better = match best {
                            None => true,
                            Some((bh, ba, br)) => {
                                h > bh + EPS || ((h - bh).abs() < EPS && (angle < ba || (angle == ba && area > (br[2] - br[0]) * (br[3] - br[1]) + EPS)))
                            }
                        };
                        if better {
                            best = Some((h, angle, r));
                        }
                    }
                }
            }
        }
    }
    best
}

/// The text centred in `r` at capital height `h`.
fn place(text: &str, h: f64, angle: u32, r: Rect) -> ([f64; 2], [f64; 2], [f64; 2]) {
    let (along, d) = (width(text) * h, drop(text) * h);
    let across = ASCENT * h + d;
    let (cx, cy) = ((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0);
    if angle == 0 {
        let pos = [cx - along / 2.0, cy - across / 2.0];
        ([cx, pos[1] + d], pos, [along, across])
    } else {
        // Turned a quarter left: the baseline runs along +y, the letters' tops toward −x.
        let pos = [cx - across / 2.0, cy - along / 2.0];
        ([pos[0] + ASCENT * h, cy], pos, [across, along])
    }
}

fn r3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// Where the top face isn't solid: pockets (a card holder's open ends run out to the edge) and
/// grab holes.
fn top_obstacles(t: &Tray, m: f64) -> Vec<Rect> {
    let mut out = vec![];
    for q in &t.pockets {
        let (mut lo, mut hi) = (q.pos, [q.pos[0] + q.size[0], q.pos[1] + q.size[1]]);
        if q.open_ends {
            let a = if q.size[0] >= q.size[1] { 0 } else { 1 };
            (lo[a], hi[a]) = (0.0, t.size[a]);
        }
        out.push([lo[0] - m, lo[1] - m, hi[0] + m, hi[1] + m]);
        out.extend(t.notch_rects(q).into_iter().map(|r| [r[0] - m, r[1] - m, r[2] + m, r[3] + m]));
    }
    if let Some(hc) = &t.half_circle {
        let r = hc.diameter / 2.0 + m;
        out.extend(hc.centers.iter().map(|c| [c[0] - r, c[1] - r, c[0] + r, c[1] + r]));
    }
    out
}

/// A pocket floor's flat part (inside any scoop fillet), and what goes through it.
fn floor_area(q: &Pocket, m: f64) -> (Rect, Vec<Rect>) {
    let [px, py] = q.pos;
    let [ql, qw] = q.size;
    let region = match q.shape {
        Shape::Rect => {
            let i = q.scoop_radius().unwrap_or(0.0) + m;
            [px + i, py + i, px + ql - i, py + qw - i]
        }
        // The square inside the circle.
        Shape::Round => {
            let (c, s) = ([px + ql / 2.0, py + qw / 2.0], ql / std::f64::consts::SQRT_2 / 2.0 - m);
            [c[0] - s, c[1] - s, c[0] + s, c[1] + s]
        }
    };
    let mut holes = vec![];
    if let Some(d) = q.finger_hole_diameter() {
        let (cx, cy, r) = (px + ql / 2.0, py + qw / 2.0, d / 2.0 + m);
        holes.push([cx - r, cy - r, cx + r, cy + r]);
    }
    if let Some(([wx, wy], [wl, ww])) = q.floor_window_rect() {
        holes.push([px + wx - m, py + wy - m, px + wx + wl + m, py + wy + ww + m]);
    }
    (region, holes)
}

/// The labels a tray's tweak asks for, placed; a warning for each that has no room. A holder
/// takes its frame's pocket labels and mode unless its own tweak says otherwise.
pub fn for_tray(spec: &Spec, t: &Tray, tweak: Option<&Tweak>, frame: Option<&Tweak>, warnings: &mut Vec<Reason>) -> Vec<Label> {
    let st = &spec.rules.labels;
    let m = st.margin;
    let mode = tweak.and_then(|w| w.label_mode).or(frame.and_then(|w| w.label_mode)).unwrap_or(spec.labels.mode);
    let mode_why = match (tweak.and_then(|w| w.label_mode), frame.and_then(|w| w.label_mode)) {
        (Some(_), _) => "the tray's label_mode",
        (None, Some(_)) => "its frame's label_mode",
        _ => "the game's labels.mode",
    };
    let pocket_labels = tweak.and_then(|w| w.pocket_labels).or(frame.and_then(|w| w.pocket_labels)).unwrap_or(false);
    let mut asks: Vec<(String, String, f64, Rect, Vec<Rect>, String)> = vec![];
    if let Some(text) = tweak.and_then(|w| w.label.clone()) {
        if t.frame {
            warnings.push(Reason::new("label_no_room", format!("{}: a frame's top is its walls and finger slots, so the label '{text}' is left off; label its holders' pockets instead", t.id)).subject(&t.id));
        } else {
            let region = [m, m, t.size[0] - m, t.size[1] - m];
            asks.push((text, "top".into(), t.size[2], region, top_obstacles(t, m), "the tray's top".into()));
        }
    }
    if pocket_labels && !t.frame {
        for q in t.pockets.iter().filter(|q| q.items > 0 && !q.label.is_empty()) {
            let z = t.size[2] - q.depth;
            if z < st.depth + 1.0 - EPS {
                warnings.push(Reason::new("label_no_room", format!("{}: the floor under '{}' is {z:.1} mm, too thin to cut a {} mm label into", t.id, q.label, st.depth)).subject(&t.id));
                continue;
            }
            let (region, holes) = floor_area(q, m);
            let text = spec.component(&q.component).and_then(|c| c.label.clone()).unwrap_or_else(|| q.label.clone());
            asks.push((text, q.id.clone(), z, region, holes, format!("the floor of {}", q.id)));
        }
    }
    let mut out = vec![];
    for (text, on, z, region, obstacles, face) in asks {
        let spot = (region[2] > region[0] && region[3] > region[1]).then(|| best_spot(region, &obstacles, &text, st.height)).flatten();
        let Some((h, angle, r)) = spot.filter(|s| s.0 >= st.min_height - EPS) else {
            let got = spot.map_or("no clear spot".into(), |s| format!("room for {:.1} mm capitals", s.0));
            warnings.push(
                Reason::new("label_no_room", format!("{}: no room for the label '{text}' on {face}: {got}, under the style's {} mm (labels.min_height)", t.id, st.min_height))
                    .subject(&t.id),
            );
            continue;
        };
        let (at, pos, size) = place(&text, h, angle, r);
        let sized = if h + EPS >= st.height { format!("{h} mm capitals (style labels.height)") } else { format!("{h} mm capitals, the most the clear spot takes (style labels.height is {})", st.height) };
        let way = if angle == 0 { "along the tray's length" } else { "across, reading up its width" };
        out.push(Label {
            text,
            on,
            mode,
            z: r3(z),
            depth: st.depth,
            height: h,
            angle,
            at: at.map(r3),
            pos: pos.map(r3),
            size: size.map(r3),
            why: format!(
                "{sized}, {way}, centred in the largest clear {:.1} × {:.1} mm on {face} ({m} mm from every edge, pocket and hole); cut {} mm deep, {} ({mode_why})",
                r[2] - r[0],
                r[3] - r[1],
                st.depth,
                if mode == LabelMode::Inlay { "inlaid with its own part" } else { "left open" }
            ),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_follow_solidcraft() {
        // "Tokens" at 8 mm measured 37.45 mm of ink in SolidCraft; the advances run a little wide.
        let w = width("Tokens") * 8.0;
        assert!((37.4..38.5).contains(&w), "{w}");
    }

    #[test]
    fn text_goes_where_it_is_biggest() {
        // A 60 × 20 strip: along it, 8 mm capitals fit; across, far less.
        let (h, angle, _) = best_spot([0.0, 0.0, 60.0, 20.0], &[], "Cards", 8.0).unwrap();
        assert_eq!((h, angle), (8.0, 0));
        // A 20 × 60 strip: the same text turns to read along it.
        let (h, angle, _) = best_spot([0.0, 0.0, 20.0, 60.0], &[], "Cards", 8.0).unwrap();
        assert_eq!((h, angle), (8.0, 90));
    }

    #[test]
    fn obstacles_are_kept_clear() {
        // A hole in the middle of a 100 × 40 face leaves strips either side.
        let hole = [30.0, 0.0, 70.0, 40.0];
        let (h, angle, r) = best_spot([0.0, 0.0, 100.0, 40.0], &[hole], "Rules", 20.0).unwrap();
        assert!(!overlaps(r, hole));
        // A 30 × 40 strip: "Rules" is bigger reading up its 40 mm than along its 30.
        assert_eq!((h, angle), (10.7, 90));
        let (at, pos, size) = place("Rules", h, angle, r);
        assert!(pos[0] >= r[0] - EPS && pos[0] + size[0] <= r[2] + EPS && pos[1] >= r[1] - EPS && pos[1] + size[1] <= r[3] + EPS);
        assert!((at[1] - (r[1] + r[3]) / 2.0).abs() < EPS);
    }
}
