//! Measuring a piece from a photo of it on a printed scan mat (PLAN M12, phase 1).
//!
//! The mats (`web/mats/`, made by `tools/scanmat/scanmat.py`) carry ArUco markers at known
//! places round a coloured field. The corner markers are found from the field (its hue's largest
//! patch, and that patch's corners), or the user taps them; each is pulled onto its marker's
//! centre, and the four give the homography from the mat (mm) to the photo. The field's colour is keyed out, the largest thing left is the piece, and a box is
//! fitted to its outline. A piece's top is nearer the camera than the mat, so it looks larger:
//! the camera's height (from the homography and the photo's EXIF focal length) and the piece's
//! own height correct the fit back to the footprint. Every shot gets an overlay image (the cut-out,
//! the fitted box, the numbers) for the user and the agent to check it by.

use std::path::{Path, PathBuf};

use image::{Rgb, RgbImage};
use serde::Deserialize;

use crate::Reason;

/// Diagonal of a 35 mm frame, mm: a "35 mm-equivalent" focal length is relative to it.
const FRAME_DIAGONAL: f64 = 43.266_615;
/// A phone's main (1×) lens, 35 mm-equivalent, when the photo doesn't say.
const ASSUMED_FOCAL_35: f64 = 26.0;

#[derive(Deserialize)]
struct MatsFile {
    mats: Vec<Mat>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Mat {
    pub key: String,
    pub name: String,
    pub file: String,
    #[serde(default)]
    pub printed_rgb: Option<[u8; 3]>,
    /// x, y, width, height on the page, mm.
    pub field: [f64; 4],
    pub markers: Vec<Marker>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Marker {
    pub centre: [f64; 2],
    pub size: f64,
}

pub fn mats() -> Vec<Mat> {
    serde_json::from_str::<MatsFile>(include_str!("../../../web/mats/mats.json")).expect("web/mats/mats.json").mats
}

pub fn mat(key: &str) -> Result<Mat, Reason> {
    let all = mats();
    let keys: Vec<String> = all.iter().map(|m| m.key.clone()).collect();
    all.into_iter()
        .find(|m| m.key == key)
        .ok_or_else(|| Reason::new("bad_mat", format!("no mat '{key}': {}", keys.join(", "))).subject(key))
}

impl Mat {
    /// The corner markers, top-left, top-right, bottom-right, bottom-left (page mm, y down).
    pub fn corners(&self) -> [&Marker; 4] {
        let by = |f: &dyn Fn(&Marker) -> f64| self.markers.iter().min_by(|a, b| f(a).total_cmp(&f(b))).unwrap();
        [
            by(&|m| m.centre[0] + m.centre[1]),
            by(&|m| -m.centre[0] + m.centre[1]),
            by(&|m| -m.centre[0] - m.centre[1]),
            by(&|m| m.centre[0] - m.centre[1]),
        ]
    }
}

pub fn overlay_path(games: &Path, slug: &str, scan: &str) -> PathBuf {
    games.join(slug).join(".build").join("scans").join(format!("{scan}.jpg"))
}

// ---------- EXIF ----------

/// The 35 mm-equivalent focal length a JPEG's EXIF records (tag 0xA405), if any.
pub fn focal_35(bytes: &[u8]) -> Option<f64> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }
    let mut i = 2;
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xFF {
            return None;
        }
        let m = bytes[i + 1];
        if m == 0xFF {
            i += 1;
            continue;
        }
        if m == 0x01 || (0xD0..=0xD8).contains(&m) {
            i += 2;
            continue;
        }
        if m == 0xDA || m == 0xD9 {
            return None;
        }
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        let seg = bytes.get(i + 4..i + 2 + len)?;
        if m == 0xE1 && seg.starts_with(b"Exif\0\0") {
            if let Some(f) = tiff_focal_35(&seg[6..]) {
                return Some(f);
            }
        }
        i += 2 + len;
    }
    None
}

fn tiff_focal_35(t: &[u8]) -> Option<f64> {
    let le = match t.get(0..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let u16_at = |o: usize| -> Option<u16> {
        let b = t.get(o..o + 2)?;
        Some(if le { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) })
    };
    let u32_at = |o: usize| -> Option<u32> {
        let b = t.get(o..o + 4)?;
        let a = [b[0], b[1], b[2], b[3]];
        Some(if le { u32::from_le_bytes(a) } else { u32::from_be_bytes(a) })
    };
    let entry = |ifd: usize, tag: u16| -> Option<usize> {
        let n = u16_at(ifd)? as usize;
        (0..n).map(|k| ifd + 2 + 12 * k).find(|&e| u16_at(e) == Some(tag))
    };
    let ifd0 = u32_at(4)? as usize;
    let exif = u32_at(entry(ifd0, 0x8769)? + 8)? as usize;
    let e = entry(exif, 0xA405)?;
    let v = u16_at(e + 8)? as f64;
    (v > 0.0).then_some(v)
}

// ---------- projective geometry ----------

/// A 3×3 projective map, row-major, h[8] = 1.
pub type Homography = [f64; 9];

/// The homography taking each `src` point to its `dst` point.
pub fn homography(src: &[[f64; 2]; 4], dst: &[[f64; 2]; 4]) -> Option<Homography> {
    let mut a = [[0.0f64; 9]; 8];
    for k in 0..4 {
        let ([x, y], [u, v]) = (src[k], dst[k]);
        a[2 * k] = [x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y, u];
        a[2 * k + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y, v];
    }
    for c in 0..8 {
        let p = (c..8).max_by(|&i, &j| a[i][c].abs().total_cmp(&a[j][c].abs()))?;
        if a[p][c].abs() < 1e-12 {
            return None;
        }
        a.swap(c, p);
        for r in 0..8 {
            if r != c {
                let f = a[r][c] / a[c][c];
                for k in c..9 {
                    a[r][k] -= f * a[c][k];
                }
            }
        }
    }
    let mut h = [0.0; 9];
    for i in 0..8 {
        h[i] = a[i][8] / a[i][i];
    }
    h[8] = 1.0;
    Some(h)
}

/// The least-squares homography taking `src` to `dst` (at least 4 pairs), normalised for
/// conditioning (Hartley).
fn homography_ls(src: &[[f64; 2]], dst: &[[f64; 2]]) -> Option<Homography> {
    let norm = |p: &[[f64; 2]]| -> Homography {
        let n = p.len() as f64;
        let (mx, my) = (p.iter().map(|q| q[0]).sum::<f64>() / n, p.iter().map(|q| q[1]).sum::<f64>() / n);
        let d = p.iter().map(|q| (q[0] - mx).hypot(q[1] - my)).sum::<f64>() / n;
        let k = std::f64::consts::SQRT_2 / d.max(1e-9);
        [k, 0.0, -k * mx, 0.0, k, -k * my, 0.0, 0.0, 1.0]
    };
    let (ts, td) = (norm(src), norm(dst));
    let mut ata = [[0.0f64; 8]; 8];
    let mut atb = [0.0f64; 8];
    for (a, b) in src.iter().zip(dst) {
        let (x, y) = apply(&ts, a[0], a[1]);
        let (u, v) = apply(&td, b[0], b[1]);
        for (row, rhs) in [([x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y], u), ([0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y], v)] {
            for i in 0..8 {
                atb[i] += row[i] * rhs;
                for j in 0..8 {
                    ata[i][j] += row[i] * row[j];
                }
            }
        }
    }
    // Solve by Gaussian elimination.
    let mut m: Vec<[f64; 9]> = (0..8).map(|i| std::array::from_fn(|j| if j < 8 { ata[i][j] } else { atb[i] })).collect();
    for c in 0..8 {
        let p = (c..8).max_by(|&i, &j| m[i][c].abs().total_cmp(&m[j][c].abs()))?;
        if m[p][c].abs() < 1e-12 {
            return None;
        }
        m.swap(c, p);
        for r in 0..8 {
            if r != c {
                let f = m[r][c] / m[c][c];
                for k in c..9 {
                    m[r][k] -= f * m[c][k];
                }
            }
        }
    }
    let hn: Homography = std::array::from_fn(|i| if i < 8 { m[i][8] / m[i][i] } else { 1.0 });
    // H = Td⁻¹ · Hn · Ts.
    let h = mul(&mul(&invert(&td)?, &hn), &ts);
    Some(h.map(|v| v / h[8]))
}

fn mul(a: &Homography, b: &Homography) -> Homography {
    std::array::from_fn(|k| {
        let (i, j) = (k / 3, k % 3);
        (0..3).map(|n| a[i * 3 + n] * b[n * 3 + j]).sum()
    })
}

pub fn apply(h: &Homography, x: f64, y: f64) -> (f64, f64) {
    let w = h[6] * x + h[7] * y + h[8];
    ((h[0] * x + h[1] * y + h[2]) / w, (h[3] * x + h[4] * y + h[5]) / w)
}

fn invert(h: &Homography) -> Option<Homography> {
    let [a, b, c, d, e, f, g, i, j] = *h;
    let det = a * (e * j - f * i) - b * (d * j - f * g) + c * (d * i - e * g);
    if det.abs() < 1e-15 {
        return None;
    }
    let m = [e * j - f * i, c * i - b * j, b * f - c * e, f * g - d * j, a * j - c * g, c * d - a * f, d * i - e * g, b * g - a * i, a * e - b * d];
    let k = 1.0 / (m[8] / det);
    Some(m.map(|v| v / det * k))
}

/// Where the camera was, in mat mm: (x, y) under it and its height above the mat, from the
/// homography mat→photo and the focal length in pixels (principal point at the image centre).
fn camera(h: &Homography, f: f64, cx: f64, cy: f64) -> ([f64; 2], f64) {
    // K⁻¹ H, column by column.
    let col = |k: usize| [(h[k] - cx * h[6 + k]) / f, (h[3 + k] - cy * h[6 + k]) / f, h[6 + k]];
    let (m1, m2, m3) = (col(0), col(1), col(2));
    let norm = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    let mut lambda = 2.0 / (norm(m1) + norm(m2));
    if m3[2] < 0.0 {
        lambda = -lambda;
    }
    let s = |v: [f64; 3]| v.map(|x| x * lambda);
    let (r1, r2, t) = (s(m1), s(m2), s(m3));
    let r3 = [r1[1] * r2[2] - r1[2] * r2[1], r1[2] * r2[0] - r1[0] * r2[2], r1[0] * r2[1] - r1[1] * r2[0]];
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    ([-dot(r1, t), -dot(r2, t)], dot(r3, t).abs())
}

// ---------- marker refinement ----------

fn gray(img: &RgbImage, x: u32, y: u32) -> u8 {
    let p = img.get_pixel(x, y).0;
    ((p[0] as u32 * 299 + p[1] as u32 * 587 + p[2] as u32 * 114) / 1000) as u8
}

/// Pull a tapped point (px) onto the centre of the marker under it, `size` px across: the dark
/// blob there (Otsu threshold in a window round the tap) and its outline's centroid. None when
/// nothing marker-like is there.
fn refine_marker(img: &RgbImage, tap: (f64, f64), size: f64) -> Option<(f64, f64)> {
    let r = (size * 0.8).max(6.0);
    let (x0, y0) = ((tap.0 - r).max(0.0) as u32, (tap.1 - r).max(0.0) as u32);
    let (x1, y1) = (((tap.0 + r) as u32).min(img.width() - 1), ((tap.1 + r) as u32).min(img.height() - 1));
    if x1 <= x0 + 4 || y1 <= y0 + 4 {
        return None;
    }
    let (w, h) = ((x1 - x0 + 1) as usize, (y1 - y0 + 1) as usize);
    let g: Vec<u8> = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| gray(img, x0 + x as u32, y0 + y as u32)).collect();
    // Otsu.
    let mut hist = [0u64; 256];
    for &v in &g {
        hist[v as usize] += 1;
    }
    let total = g.len() as f64;
    let sum: f64 = hist.iter().enumerate().map(|(i, &n)| i as f64 * n as f64).sum();
    let (mut wb, mut sb, mut best, mut thr) = (0.0, 0.0, -1.0, 128u8);
    for (t, &n) in hist.iter().enumerate() {
        wb += n as f64;
        if wb == 0.0 || wb == total {
            continue;
        }
        sb += t as f64 * n as f64;
        let (mb, mf) = (sb / wb, (sum - sb) / (total - wb));
        let between = wb * (total - wb) * (mb - mf).powi(2);
        if between > best {
            best = between;
            thr = t as u8;
        }
    }
    let dark: Vec<bool> = g.iter().map(|&v| v <= thr).collect();
    // The largest dark blob that reaches near the tap: the marker's black border and bits.
    let mut label = vec![0u32; w * h];
    let (mut best_n, mut best_l) = (0usize, 0u32);
    let mut next = 0u32;
    let mut stack = vec![];
    let near = (size * 0.5).powi(2);
    for start in 0..w * h {
        if !dark[start] || label[start] != 0 {
            continue;
        }
        next += 1;
        let (mut n, mut close) = (0usize, false);
        stack.push(start);
        label[start] = next;
        while let Some(i) = stack.pop() {
            n += 1;
            let (x, y) = (i % w, i / w);
            let (px, py) = (x0 as f64 + x as f64, y0 as f64 + y as f64);
            close |= (px - tap.0).powi(2) + (py - tap.1).powi(2) < near;
            let mut push = |j: usize| {
                if dark[j] && label[j] == 0 {
                    label[j] = next;
                    stack.push(j);
                }
            };
            if x > 0 { push(i - 1); }
            if x + 1 < w { push(i + 1); }
            if y > 0 { push(i - w); }
            if y + 1 < h { push(i + w); }
        }
        if close && n > best_n {
            (best_n, best_l) = (n, next);
        }
    }
    if best_l == 0 {
        return None;
    }
    let pts: Vec<[f64; 2]> = row_extremes(w, h, |i| label[i] == best_l).into_iter().map(|[x, y]| [x + x0 as f64, y + y0 as f64]).collect();
    let hull = convex_hull(pts);
    let (bx0, bx1) = hull.iter().fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p[0]), b.max(p[0])));
    let (by0, by1) = hull.iter().fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p[1]), b.max(p[1])));
    // A marker is about `size` across; a blob much smaller or larger is something else.
    let span = (bx1 - bx0).max(by1 - by0);
    if span < size * 0.6 || span > size * 1.6 {
        return None;
    }
    polygon_centroid(&hull)
}

/// The left and right edge points of each row of a mask, as pixel-corner coordinates.
fn row_extremes(w: usize, h: usize, inside: impl Fn(usize) -> bool) -> Vec<[f64; 2]> {
    let mut pts = vec![];
    for y in 0..h {
        let row = y * w;
        let Some(a) = (0..w).find(|&x| inside(row + x)) else { continue };
        let b = (0..w).rev().find(|&x| inside(row + x)).unwrap();
        for yy in [y as f64, y as f64 + 1.0] {
            pts.push([a as f64, yy]);
            pts.push([b as f64 + 1.0, yy]);
        }
    }
    pts
}

fn convex_hull(mut p: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    p.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    p.dedup();
    if p.len() < 3 {
        return p;
    }
    let cross = |o: [f64; 2], a: [f64; 2], b: [f64; 2]| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    let mut lower: Vec<[f64; 2]> = vec![];
    for &q in &p {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], q) <= 0.0 {
            lower.pop();
        }
        lower.push(q);
    }
    let mut upper: Vec<[f64; 2]> = vec![];
    for &q in p.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], q) <= 0.0 {
            upper.pop();
        }
        upper.push(q);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

fn polygon_centroid(p: &[[f64; 2]]) -> Option<(f64, f64)> {
    let (mut a, mut cx, mut cy) = (0.0, 0.0, 0.0);
    for i in 0..p.len() {
        let (q, r) = (p[i], p[(i + 1) % p.len()]);
        let c = q[0] * r[1] - r[0] * q[1];
        a += c;
        cx += (q[0] + r[0]) * c;
        cy += (q[1] + r[1]) * c;
    }
    (a.abs() > 1e-9).then(|| (cx / (3.0 * a), cy / (3.0 * a)))
}

/// Taps on the four corner markers, in any order round the mat, put in the mat's order
/// (top-left, top-right, bottom-right, bottom-left, long edges first) as the photo shows it.
fn order_taps(t: [(f64, f64); 4]) -> [(f64, f64); 4] {
    // Round the centre by angle: then they go round, whatever order they were tapped in.
    let (mx, my) = (t.iter().map(|p| p.0).sum::<f64>() / 4.0, t.iter().map(|p| p.1).sum::<f64>() / 4.0);
    let mut t = t;
    t.sort_by(|a, b| (a.1 - my).atan2(a.0 - mx).total_cmp(&(b.1 - my).atan2(b.0 - mx)));
    // atan2 with y down goes clockwise on screen, as the mat's TL → TR → BR does.
    let d = |a: (f64, f64), b: (f64, f64)| ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
    if d(t[0], t[1]) + d(t[2], t[3]) >= d(t[1], t[2]) + d(t[3], t[0]) { t } else { [t[3], t[0], t[1], t[2]] }
}

// ---------- finding the mat ----------

/// The corner markers' centres (px, in the mat's order), found without taps: the coloured field
/// is the largest patch of its hue, its outline's largest quadrilateral is the field's corners,
/// and those put each corner marker where it is looked for. An error says what wasn't found.
fn find_corners(img: &RgbImage, mat: &Mat, s: f64) -> Result<[(f64, f64); 4], String> {
    let Some(rgb) = mat.printed_rgb else { return Err("this mat has no field colour".into()) };
    let want = hsv(rgb);
    let (pw, ph) = (img.width() as f64, img.height() as f64);
    let k = (640.0 / pw.max(ph)).min(1.0);
    let (w, h) = (((pw * k).round() as u32).max(1), ((ph * k).round() as u32).max(1));
    let small = image::imageops::thumbnail(img, w, h);
    let (w, h) = (small.width() as usize, small.height() as usize);
    // The field's hue, lit or in shade.
    let mask: Vec<bool> = small.pixels().map(|p| {
        let c = hsv(p.0);
        hue_diff(c.h, want.h) < 28.0 && c.s > 0.18 && c.v > 0.07
    }).collect();
    let mask = erode(&dilate(&mask, w, h, 2), w, h, 2);
    let mask = dilate(&erode(&mask, w, h, 1), w, h, 1);
    let (blob, n) = largest_blob(&mask, w, h).ok_or("no patch of the mat's colour")?;
    if (n as f64) < (w * h) as f64 * 0.03 {
        return Err("too little of the mat's colour: is it the right mat, and in the frame?".into());
    }
    let mut hull = convex_hull(row_extremes(w, h, |i| blob[i]));
    if hull.len() < 4 {
        return Err("the field has no corners".into());
    }
    if hull.len() > 120 {
        let step = hull.len() as f64 / 120.0;
        hull = (0..120).map(|i| hull[(i as f64 * step) as usize]).collect();
    }
    let quad = max_quad(&hull);
    let quad = order_taps(quad.map(|[x, y]| (x / k, y / k)));
    let [fx, fy, fw, fh] = mat.field.map(|v| v * s);
    let field = [[fx, fy], [fx + fw, fy], [fx + fw, fy + fh], [fx, fy + fh]];
    let hf = homography(&field, &quad.map(|(x, y)| [x, y])).ok_or("the field isn't a quadrilateral")?;
    let corners = mat.corners();
    let mut found = [(0.0, 0.0); 4];
    for (i, m) in corners.iter().enumerate() {
        let (cx, cy) = (m.centre[0] * s, m.centre[1] * s);
        let guess = apply(&hf, cx, cy);
        let a = apply(&hf, cx - m.size * s / 2.0, cy);
        let b = apply(&hf, cx + m.size * s / 2.0, cy);
        let size = (a.0 - b.0).hypot(a.1 - b.1);
        if guess.0 < 0.0 || guess.1 < 0.0 || guess.0 >= pw || guess.1 >= ph {
            return Err("a corner marker is out of the frame".into());
        }
        found[i] = refine_marker(img, guess, size)
            .filter(|p| (p.0 - guess.0).hypot(p.1 - guess.1) < size * 0.6)
            .ok_or("a corner marker wasn't where the field put it")?;
    }
    Ok(found)
}

/// The largest-area quadrilateral on a convex polygon's vertices.
fn max_quad(p: &[[f64; 2]]) -> [[f64; 2]; 4] {
    let n = p.len();
    let tri = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])).abs() / 2.0;
    let (mut best, mut q) = (-1.0, [0, 1, 2, 3]);
    for i in 0..n {
        for k in i + 2..n {
            let (mut bj, mut aj) = (i + 1, -1.0);
            for j in i + 1..k {
                let a = tri(p[i], p[j], p[k]);
                if a > aj { (aj, bj) = (a, j); }
            }
            let (mut bl, mut al) = (k, -1.0);
            for l in (k + 1..n).chain(0..i) {
                let a = tri(p[i], p[k], p[l]);
                if a > al { (al, bl) = (a, l); }
            }
            if al >= 0.0 && aj + al > best {
                best = aj + al;
                q = [i, bj, k, bl];
            }
        }
    }
    q.map(|i| p[i])
}

// ---------- measuring ----------

/// What a scan says, for the spec.
#[derive(Clone, Debug)]
pub struct Measured {
    /// The corner markers' centres as found, 0–1 of the photo, TL TR BR BL.
    pub corners: [[f64; 2]; 4],
    /// Footprint, long × short side, mm, corrected for the piece's height.
    pub size: [f64; 2],
    /// The outline's box as it lies on the mat, uncorrected.
    pub raw: [f64; 2],
    /// Cut-out area ÷ its box's area.
    pub fill: f64,
    pub camera_height: f64,
    pub focal_35: f64,
    pub focal_assumed: bool,
    /// The cut-out's hull, mm, about the footprint's centre, long side along x.
    pub outline: Vec<[f64; 2]>,
    /// The footprint's top face (at the piece's height) as photo corners, 0–1, for the top face.
    pub top_face: [[f64; 2]; 4],
    pub notes: Vec<String>,
}

pub struct ScanInput<'a> {
    pub photo: &'a Path,
    pub mat: &'a Mat,
    /// The four corner markers as tapped, 0–1 of the photo; None finds them.
    pub taps: Option<[[f64; 2]; 4]>,
    pub focal_35: Option<f64>,
    /// How tall the piece stands in this shot, mm (None: not corrected).
    pub height: Option<f64>,
    /// The mat as printed ÷ as drawn (a printer that fits the page prints it small).
    pub print_scale: f64,
    pub overlay: &'a Path,
}

fn bad(msg: impl Into<String>) -> Reason {
    Reason::new("scan_failed", msg.into())
}

pub fn measure(input: &ScanInput) -> Result<Measured, Reason> {
    let mat = input.mat;
    let Some(field_rgb) = mat.printed_rgb else {
        return Err(Reason::new("white_mat", format!("the {} mat has no field colour to key out: it's for checking the print size. Measure on Teal, or Magenta for green, teal or blue pieces", mat.name)));
    };
    let img = image::open(input.photo).map_err(|e| bad(format!("can't read the photo: {e}")))?.to_rgb8();
    let (pw, ph) = (img.width() as f64, img.height() as f64);
    let s = input.print_scale;
    let corners = mat.corners();
    let mat_pts: [[f64; 2]; 4] = corners.map(|m| [m.centre[0] * s, m.centre[1] * s]);
    let tapped = input.taps.is_some();
    let taps = match input.taps {
        Some(t) => order_taps(t.map(|[x, y]| (x * pw, y * ph))),
        None => find_corners(&img, mat, s).map_err(|why| Reason::new("no_corners", format!("couldn't find the mat's corners by itself ({why}): tap the four corner markers")))?,
    };
    let h0 = homography(&mat_pts, &taps.map(|(x, y)| [x, y])).ok_or_else(|| bad("the four taps don't make a quadrilateral: tap each corner marker once"))?;
    // Each tap onto its marker's centre, sized from the first guess.
    let mut found = 0;
    let refined: [(f64, f64); 4] = std::array::from_fn(|k| {
        let m = corners[k];
        let half = m.size * s / 2.0;
        let a = apply(&h0, m.centre[0] * s - half, m.centre[1] * s);
        let b = apply(&h0, m.centre[0] * s + half, m.centre[1] * s);
        let px = ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
        match refine_marker(&img, taps[k], px) {
            Some(p) if (p.0 - taps[k].0).hypot(p.1 - taps[k].1) < px * 0.6 => {
                found += 1;
                p
            }
            _ => taps[k],
        }
    });
    let mut notes = vec![];
    if found < 4 && tapped {
        notes.push(format!("{} of the 4 taps weren't on a marker I could find, so they're used as tapped: tap the middle of each corner marker", 4 - found));
    }
    let h4 = homography(&mat_pts, &refined.map(|(x, y)| [x, y])).ok_or_else(|| bad("the markers don't make a quadrilateral"))?;
    // The other markers (the middles of the long edges), found where the corners put them, make
    // the fit a least-squares one.
    let (mut src, mut dst): (Vec<[f64; 2]>, Vec<[f64; 2]>) = (mat_pts.to_vec(), refined.iter().map(|&(x, y)| [x, y]).collect());
    for m in mat.markers.iter().filter(|m| !corners.iter().any(|c| std::ptr::eq(*c, *m))) {
        let (cx, cy) = (m.centre[0] * s, m.centre[1] * s);
        let guess = apply(&h4, cx, cy);
        let a = apply(&h4, cx - m.size * s / 2.0, cy);
        let b = apply(&h4, cx + m.size * s / 2.0, cy);
        let size = (a.0 - b.0).hypot(a.1 - b.1);
        if guess.0 < 0.0 || guess.1 < 0.0 || guess.0 >= pw || guess.1 >= ph {
            continue;
        }
        if let Some(p) = refine_marker(&img, guess, size).filter(|p| (p.0 - guess.0).hypot(p.1 - guess.1) < size * 0.4) {
            src.push([cx, cy]);
            dst.push([p.0, p.1]);
        }
    }
    let h = if src.len() > 4 { homography_ls(&src, &dst).unwrap_or(h4) } else { h4 };
    // How well the markers sit on one plane: a crumpled or curled mat shows here.
    let hi = invert(&h).ok_or_else(|| bad("the markers don't make a plane"))?;
    let residual = src.iter().zip(&dst).map(|(a, b)| {
        let (x, y) = apply(&hi, b[0], b[1]);
        (x - a[0]).hypot(y - a[1])
    }).fold(0.0f64, f64::max);
    if residual > 1.0 {
        notes.push(format!("the markers are up to {residual:.1} mm off one flat plane: lay the mat flat, or check the taps"));
    }
    let (f35, assumed) = match input.focal_35 {
        Some(f) => (f, false),
        None => {
            notes.push(format!("the photo doesn't say its focal length (send the original file, not a screenshot or an edited copy), so a 1× phone lens ({ASSUMED_FOCAL_35} mm-equivalent) is assumed"));
            (ASSUMED_FOCAL_35, true)
        }
    };
    let f_px = f35 * (pw * pw + ph * ph).sqrt() / FRAME_DIAGONAL;
    let (under, cam_h) = camera(&h, f_px, pw / 2.0, ph / 2.0);

    // The field, straightened, at about the photo's own resolution there.
    let [fx, fy, fw, fhh] = mat.field.map(|v| v * s);
    let local = {
        let (cx, cy) = (fx + fw / 2.0, fy + fhh / 2.0);
        let (a, b) = (apply(&h, cx, cy), apply(&h, cx + 10.0, cy));
        ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt() / 10.0
    };
    let ppm = local.clamp(3.0, 12.0);
    let inset = 1.5;
    let (ox, oy) = (fx + inset, fy + inset);
    let (w, hh) = (((fw - 2.0 * inset) * ppm) as usize, ((fhh - 2.0 * inset) * ppm) as usize);
    let to_mm = |i: f64, j: f64| (ox + i / ppm, oy + j / ppm);
    let mut px = vec![[0u8; 3]; w * hh];
    for j in 0..hh {
        for i in 0..w {
            let (x, y) = to_mm(i as f64 + 0.5, j as f64 + 0.5);
            let (u, v) = apply(&h, x, y);
            px[j * w + i] = crate::media::sample(&img, u, v);
        }
    }
    // The field's colour as lit, from a band just inside its edge.
    let band = (6.0 * ppm) as usize;
    let mut ring = vec![];
    for j in 0..hh {
        for i in 0..w {
            if i < band || j < band || i + band >= w || j + band >= hh {
                ring.push(hsv(px[j * w + i]));
            }
        }
    }
    let field = field_colour(&ring).unwrap_or_else(|| hsv(field_rgb));
    let mut obj: Vec<bool> = px.iter().map(|&p| !is_field(hsv(p), field)).collect();
    let r = ((0.25 * ppm).round() as usize).max(1);
    obj = dilate(&erode(&obj, w, hh, r), w, hh, r);
    obj = erode(&dilate(&obj, w, hh, r), w, hh, r);
    let (label, n) = largest_blob(&obj, w, hh).ok_or_else(|| bad("no piece found on the field: is it on the coloured area, and is the mat the right one?"))?;
    let area_mm = n as f64 / (ppm * ppm);
    if area_mm < 4.0 {
        return Err(bad(format!("only a {area_mm:.1} mm² speck stands out from the field: is the piece on the coloured area?")));
    }
    let edge = label.iter().enumerate().any(|(k, &b)| b && (k % w == 0 || k / w == 0 || k % w == w - 1 || k / w == hh - 1));
    if edge {
        notes.push("the cut-out touches the field's edge: keep the piece well inside the coloured area".into());
    }
    let hull: Vec<[f64; 2]> = convex_hull(row_extremes(w, hh, |k| label[k]).into_iter().map(|[i, j]| {
        let (x, y) = to_mm(i, j);
        [x, y]
    }).collect());
    let (theta, (u0, u1), (v0, v1)) = min_area_rect(&hull);
    let fill = area_mm / ((u1 - u0) * (v1 - v0));
    // The top is nearer the camera: it shows enlarged about the point under the camera.
    let k = match input.height {
        Some(t) if t > 0.0 && t < cam_h * 0.8 => cam_h / (cam_h - t),
        Some(_) => {
            notes.push("the piece's height is more than the camera's: not corrected".into());
            1.0
        }
        None => {
            notes.push("no height given, so the outline isn't corrected for the piece standing above the mat: it reads a little large".into());
            1.0
        }
    };
    // A first guess, side by side: each side of the outline's box pulled back towards the
    // camera for the top's enlargement.
    let (ux, uy) = (theta.cos(), theta.sin());
    let (cu, cv) = (under[0] * ux + under[1] * uy, -under[0] * uy + under[1] * ux);
    let side = |lo: f64, hi: f64, c: f64| (if lo < c { c + (lo - c) / k } else { lo }, if hi > c { c + (hi - c) / k } else { hi });
    let ((a0, a1), (b0, b1)) = (side(u0, u1, cu), side(v0, v1, cv));
    let pt = |u: f64, v: f64| [u * ux - v * uy, u * uy + v * ux];
    let c0 = pt((a0 + a1) / 2.0, (b0 + b1) / 2.0);
    // Then the box whose silhouette (footprint and raised top) best overlaps the cut-out: the
    // outline's extremes ride on blurred edge pixels, the overlap doesn't.
    let footprint = |p: &[f64; 5]| -> [[f64; 2]; 4] {
        let [x, y, th, w, d] = *p;
        let (c, s) = (th.cos(), th.sin());
        [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(su, sv)| {
            let (u, v) = (su * w / 2.0, sv * d / 2.0);
            [x + u * c - v * s, y + u * s + v * c]
        })
    };
    let silhouette = |p: &[f64; 5]| -> Vec<[f64; 2]> {
        let f = footprint(p);
        let mut pts = f.to_vec();
        pts.extend(f.iter().map(|&[x, y]| [under[0] + (x - under[0]) * k, under[1] + (y - under[1]) * k]));
        convex_hull(pts)
    };
    let raw_box_mm = [pt(u0, v0), pt(u1, v0), pt(u1, v1), pt(u0, v1)];
    let (rx0, ry0) = raw_box_mm.iter().fold((f64::MAX, f64::MAX), |(a, b), p| (a.min(p[0]), b.min(p[1])));
    let (rx1, ry1) = raw_box_mm.iter().fold((f64::MIN, f64::MIN), |(a, b), p| (a.max(p[0]), b.max(p[1])));
    let to_px = |x: f64, y: f64| ((x - ox) * ppm, (y - oy) * ppm);
    let (pi0, pj0) = to_px(rx0 - 3.0, ry0 - 3.0);
    let (pi1, pj1) = to_px(rx1 + 3.0, ry1 + 3.0);
    let (pi0, pj0) = (pi0.max(0.0) as usize, pj0.max(0.0) as usize);
    let (pi1, pj1) = ((pi1.max(0.0) as usize).min(w), (pj1.max(0.0) as usize).min(hh));
    let loss = |p: &[f64; 5]| -> f64 {
        if p[3] <= 0.5 || p[4] <= 0.5 {
            return 1.0;
        }
        let poly: Vec<[f64; 2]> = silhouette(p).into_iter().map(|[x, y]| {
            let (i, j) = to_px(x, y);
            [i, j]
        }).collect();
        let (mut inter, mut union) = (0usize, 0usize);
        for j in pj0..pj1 {
            for i in pi0..pi1 {
                let m = label[j * w + i];
                let q = inside_convex(&poly, i as f64 + 0.5, j as f64 + 0.5);
                inter += (m && q) as usize;
                union += (m || q) as usize;
            }
        }
        1.0 - inter as f64 / union.max(1) as f64
    };
    let start = [c0[0], c0[1], theta, a1 - a0, b1 - b0];
    let best = nelder_mead(&loss, start, [0.4, 0.4, 0.03, 0.4, 0.4], 600);
    let iou = 1.0 - loss(&best);
    let [_, _, th, du, dv] = best;
    let foot = footprint(&best);
    let (ux, uy) = (th.cos(), th.sin());
    let (long_u, size, raw) = if du >= dv { (true, [du, dv], [u1 - u0, v1 - v0]) } else { (false, [dv, du], [v1 - v0, u1 - u0]) };
    if fill < 0.85 {
        notes.push(format!("the cut-out fills only {:.0}% of its box: not a rectangle, or something else is in the cut-out", fill * 100.0));
    } else if iou < 0.93 {
        notes.push(format!("the box fits the cut-out only {:.0}%: check the overlay", iou * 100.0));
    }
    let top = foot.map(|[x, y]| [under[0] + (x - under[0]) * k, under[1] + (y - under[1]) * k]);
    let top_face = top.map(|[x, y]| {
        let (u, v) = apply(&h, x, y);
        [(u / pw).clamp(0.0, 1.0), (v / ph).clamp(0.0, 1.0)]
    });
    let centre = [best[0], best[1]];
    let outline = hull
        .iter()
        .map(|&[x, y]| {
            let (dx, dy) = (x - centre[0], y - centre[1]);
            let (u, v) = (dx * ux + dy * uy, -dx * uy + dy * ux);
            let p = if long_u { [u, v] } else { [v, -u] };
            p.map(round2)
        })
        .collect();

    // The overlay: the piece and round it, straightened, with what was found drawn on.
    let raw_box = raw_box_mm;
    let (bx0, by0) = raw_box.iter().fold((f64::MAX, f64::MAX), |(a, b), p| (a.min(p[0]), b.min(p[1])));
    let (bx1, by1) = raw_box.iter().fold((f64::MIN, f64::MIN), |(a, b), p| (a.max(p[0]), b.max(p[1])));
    let margin = ((bx1 - bx0).max(by1 - by0) * 0.35).max(8.0);
    let (cx0, cy0, cx1, cy1) = (bx0 - margin, by0 - margin - 10.0, bx1 + margin, by1 + margin);
    let op = (900.0 / (cx1 - cx0).max(cy1 - cy0)).clamp(4.0, 40.0);
    let (ow, oh) = (((cx1 - cx0) * op) as u32, ((cy1 - cy0) * op) as u32);
    let mut out = RgbImage::from_fn(ow, oh, |i, j| {
        let (x, y) = (cx0 + (i as f64 + 0.5) / op, cy0 + (j as f64 + 0.5) / op);
        let (u, v) = apply(&h, x, y);
        let mut c = crate::media::sample(&img, u, v);
        let (mi, mj) = ((x - ox) * ppm, (y - oy) * ppm);
        if mi >= 0.0 && mj >= 0.0 && (mi as usize) < w && (mj as usize) < hh {
            if label[mj as usize * w + mi as usize] {
                c = blend(c, [255, 220, 0], 0.30);
            }
        } else {
            c = blend(c, [0, 0, 0], 0.35);
        }
        Rgb(c)
    });
    let to_o = |p: [f64; 2]| ((p[0] - cx0) * op, (p[1] - cy0) * op);
    let poly = |out: &mut RgbImage, pts: &[[f64; 2]], c: [u8; 3], t: f64| {
        for i in 0..pts.len() {
            line(out, to_o(pts[i]), to_o(pts[(i + 1) % pts.len()]), c, t);
        }
    };
    poly(&mut out, &hull, [255, 220, 0], 1.0);
    poly(&mut out, &raw_box, [0, 200, 255], 1.0);
    poly(&mut out, &foot, [255, 40, 200], 2.0);
    // Labels: each footprint side's length beside its middle, outside the box.
    let mid = |a: [f64; 2], b: [f64; 2]| [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
    let c_mm = mid(foot[0], foot[2]);
    for (e, len) in [((0, 1), du), ((1, 2), dv)] {
        let m = mid(foot[e.0], foot[e.1]);
        let (dx, dy) = (m[0] - c_mm[0], m[1] - c_mm[1]);
        let dl = (dx * dx + dy * dy).sqrt().max(1e-6);
        let at = to_o([m[0] + dx / dl * 5.0, m[1] + dy / dl * 5.0]);
        text(&mut out, at, &format!("{} mm", fmt(len)), 3, true);
    }
    let head = format!(
        "{} x {} mm  fill {:.0}%  cam {:.0} mm  f {:.0}{}  h {}",
        fmt(size[0]),
        fmt(size[1]),
        fill * 100.0,
        cam_h,
        f35,
        if assumed { "?" } else { "" },
        input.height.map_or("?".into(), fmt)
    );
    text(&mut out, (6.0, 6.0), &head, 2, false);
    std::fs::create_dir_all(input.overlay.parent().unwrap()).map_err(|e| bad(e.to_string()))?;
    out.save_with_format(input.overlay, image::ImageFormat::Jpeg).map_err(|e| bad(e.to_string()))?;

    Ok(Measured {
        corners: refined.map(|(x, y)| [x / pw, y / ph]),
        size: size.map(round2),
        raw: raw.map(round2),
        fill: (fill * 1000.0).round() / 1000.0,
        camera_height: cam_h.round(),
        focal_35: f35,
        focal_assumed: assumed,
        outline,
        top_face,
        notes,
    })
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn fmt(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Is (x, y) inside a convex polygon (either winding)?
fn inside_convex(p: &[[f64; 2]], x: f64, y: f64) -> bool {
    let mut sign = 0.0f64;
    for i in 0..p.len() {
        let (a, b) = (p[i], p[(i + 1) % p.len()]);
        let c = (b[0] - a[0]) * (y - a[1]) - (b[1] - a[1]) * (x - a[0]);
        if c != 0.0 {
            if sign != 0.0 && c.signum() != sign {
                return false;
            }
            sign = c.signum();
        }
    }
    true
}

/// Nelder–Mead downhill simplex: the point of least `f` near `x0`, from steps `step`.
fn nelder_mead<const N: usize>(f: &dyn Fn(&[f64; N]) -> f64, x0: [f64; N], step: [f64; N], iters: usize) -> [f64; N] {
    let mut pts: Vec<([f64; N], f64)> = (0..=N)
        .map(|i| {
            let mut x = x0;
            if i > 0 {
                x[i - 1] += step[i - 1];
            }
            (x, f(&x))
        })
        .collect();
    for _ in 0..iters {
        pts.sort_by(|a, b| a.1.total_cmp(&b.1));
        if (pts[N].1 - pts[0].1).abs() < 1e-7 && (0..N).all(|d| (pts[N].0[d] - pts[0].0[d]).abs() < 1e-3) {
            break;
        }
        let mut c = [0.0; N];
        for p in &pts[..N] {
            for d in 0..N {
                c[d] += p.0[d] / N as f64;
            }
        }
        let worst = pts[N];
        let along = |t: f64| -> [f64; N] { std::array::from_fn(|d| c[d] + t * (worst.0[d] - c[d])) };
        let xr = along(-1.0);
        let fr = f(&xr);
        if fr < pts[0].1 {
            let xe = along(-2.0);
            let fe = f(&xe);
            pts[N] = if fe < fr { (xe, fe) } else { (xr, fr) };
        } else if fr < pts[N - 1].1 {
            pts[N] = (xr, fr);
        } else {
            let xc = if fr < worst.1 { along(-0.5) } else { along(0.5) };
            let fc = f(&xc);
            if fc < worst.1.min(fr) {
                pts[N] = (xc, fc);
            } else {
                let best = pts[0].0;
                for p in pts.iter_mut().skip(1) {
                    p.0 = std::array::from_fn(|d| best[d] + (p.0[d] - best[d]) / 2.0);
                    p.1 = f(&p.0);
                }
            }
        }
    }
    pts.sort_by(|a, b| a.1.total_cmp(&b.1));
    pts[0].0
}

/// The rectangle of least area round a convex hull: its angle, and the hull's extent along it
/// and across it.
fn min_area_rect(hull: &[[f64; 2]]) -> (f64, (f64, f64), (f64, f64)) {
    let mut best = (f64::MAX, 0.0, (0.0, 0.0), (0.0, 0.0));
    for i in 0..hull.len() {
        let (a, b) = (hull[i], hull[(i + 1) % hull.len()]);
        if a == b {
            continue;
        }
        let th = (b[1] - a[1]).atan2(b[0] - a[0]);
        let (c, s) = (th.cos(), th.sin());
        let (mut u, mut v) = ((f64::MAX, f64::MIN), (f64::MAX, f64::MIN));
        for p in hull {
            let (pu, pv) = (p[0] * c + p[1] * s, -p[0] * s + p[1] * c);
            u = (u.0.min(pu), u.1.max(pu));
            v = (v.0.min(pv), v.1.max(pv));
        }
        let area = (u.1 - u.0) * (v.1 - v.0);
        if area < best.0 {
            best = (area, th, u, v);
        }
    }
    (best.1, best.2, best.3)
}

#[derive(Clone, Copy, Debug)]
struct Hsv {
    h: f64,
    s: f64,
    v: f64,
}

fn hsv(p: [u8; 3]) -> Hsv {
    let [r, g, b] = p.map(|c| c as f64 / 255.0);
    let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
    let d = mx - mn;
    let h = if d < 1e-9 {
        0.0
    } else if mx == r {
        60.0 * (((g - b) / d).rem_euclid(6.0))
    } else if mx == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    Hsv { h, s: if mx > 0.0 { d / mx } else { 0.0 }, v: mx }
}

fn hue_diff(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(360.0);
    d.min(360.0 - d)
}

/// The field's hue (circular mean) and median saturation and value from saturated samples.
fn field_colour(px: &[Hsv]) -> Option<Hsv> {
    let sat: Vec<&Hsv> = px.iter().filter(|p| p.s > 0.2 && p.v > 0.15).collect();
    if sat.len() < 50 {
        return None;
    }
    let (sx, sy) = sat.iter().fold((0.0, 0.0), |(x, y), p| (x + p.h.to_radians().cos(), y + p.h.to_radians().sin()));
    let h = sy.atan2(sx).to_degrees().rem_euclid(360.0);
    let near: Vec<&&Hsv> = sat.iter().filter(|p| hue_diff(p.h, h) < 30.0).collect();
    let med = |f: &dyn Fn(&Hsv) -> f64| {
        let mut v: Vec<f64> = near.iter().map(|p| f(p)).collect();
        v.sort_by(f64::total_cmp);
        v.get(v.len() / 2).copied()
    };
    Some(Hsv { h, s: med(&|p| p.s)?, v: med(&|p| p.v)? })
}

/// Field, or the field in shadow: its hue, saturated enough. The piece's own shadow on the field
/// keeps the field's hue, so it counts as field.
fn is_field(p: Hsv, f: Hsv) -> bool {
    let dh = hue_diff(p.h, f.h);
    (dh < 22.0 && p.s > f.s * 0.45 && p.v > 0.06) || (dh < 35.0 && p.s > f.s * 0.3 && p.v < f.v * 0.5 && p.v > 0.04)
}

fn erode(m: &[bool], w: usize, h: usize, r: usize) -> Vec<bool> {
    morph(m, w, h, r, true)
}

fn dilate(m: &[bool], w: usize, h: usize, r: usize) -> Vec<bool> {
    morph(m, w, h, r, false)
}

/// A square min (erode) or max (dilate) of radius r, separably.
fn morph(m: &[bool], w: usize, h: usize, r: usize, all: bool) -> Vec<bool> {
    let pass = |src: &[bool], horizontal: bool| -> Vec<bool> {
        let mut out = vec![false; w * h];
        for y in 0..h {
            for x in 0..w {
                let (lo, hi, n) = if horizontal { (x.saturating_sub(r), (x + r).min(w - 1), 0) } else { (y.saturating_sub(r), (y + r).min(h - 1), 1) };
                let at = |k: usize| if n == 0 { src[y * w + k] } else { src[k * w + x] };
                out[y * w + x] = if all { (lo..=hi).all(at) } else { (lo..=hi).any(at) };
            }
        }
        out
    };
    pass(&pass(m, true), false)
}

/// The largest 4-connected blob: its mask and size.
fn largest_blob(m: &[bool], w: usize, h: usize) -> Option<(Vec<bool>, usize)> {
    let mut label = vec![0u32; w * h];
    let (mut best_n, mut best_l, mut next) = (0usize, 0u32, 0u32);
    let mut stack = vec![];
    for start in 0..w * h {
        if !m[start] || label[start] != 0 {
            continue;
        }
        next += 1;
        label[start] = next;
        stack.push(start);
        let mut n = 0;
        while let Some(i) = stack.pop() {
            n += 1;
            let (x, y) = (i % w, i / w);
            for (ok, j) in [(x > 0, i.wrapping_sub(1)), (x + 1 < w, i + 1), (y > 0, i.wrapping_sub(w)), (y + 1 < h, i + w)] {
                if ok && m[j] && label[j] == 0 {
                    label[j] = next;
                    stack.push(j);
                }
            }
        }
        if n > best_n {
            (best_n, best_l) = (n, next);
        }
    }
    (best_l > 0).then(|| (label.iter().map(|&l| l == best_l).collect(), best_n))
}

// ---------- drawing ----------

fn blend(c: [u8; 3], to: [u8; 3], a: f64) -> [u8; 3] {
    std::array::from_fn(|i| (c[i] as f64 * (1.0 - a) + to[i] as f64 * a).round() as u8)
}

fn dot(img: &mut RgbImage, x: f64, y: f64, c: [u8; 3], r: f64) {
    let ri = r.ceil() as i64;
    for dy in -ri..=ri {
        for dx in -ri..=ri {
            if (dx * dx + dy * dy) as f64 <= r * r + 0.25 {
                let (px, py) = (x.round() as i64 + dx, y.round() as i64 + dy);
                if px >= 0 && py >= 0 && (px as u32) < img.width() && (py as u32) < img.height() {
                    img.put_pixel(px as u32, py as u32, Rgb(c));
                }
            }
        }
    }
}

fn line(img: &mut RgbImage, a: (f64, f64), b: (f64, f64), c: [u8; 3], r: f64) {
    let n = ((b.0 - a.0).abs().max((b.1 - a.1).abs()) * 2.0).ceil().max(1.0) as usize;
    for k in 0..=n {
        let t = k as f64 / n as f64;
        dot(img, a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, c, r);
    }
}

/// 5×7 glyphs for the overlay's labels: each row is 5 bits, high bit left.
fn glyph(ch: char) -> [u8; 7] {
    match ch {
        '0' => [14, 17, 19, 21, 25, 17, 14],
        '1' => [4, 12, 4, 4, 4, 4, 14],
        '2' => [14, 17, 1, 2, 4, 8, 31],
        '3' => [31, 2, 4, 2, 1, 17, 14],
        '4' => [2, 6, 10, 18, 31, 2, 2],
        '5' => [31, 16, 30, 1, 1, 17, 14],
        '6' => [6, 8, 16, 30, 17, 17, 14],
        '7' => [31, 1, 2, 4, 8, 8, 8],
        '8' => [14, 17, 17, 14, 17, 17, 14],
        '9' => [14, 17, 17, 15, 1, 2, 12],
        '.' => [0, 0, 0, 0, 0, 12, 12],
        '%' => [24, 25, 2, 4, 8, 19, 3],
        '?' => [14, 17, 1, 2, 4, 0, 4],
        'x' => [0, 0, 17, 10, 4, 10, 17],
        'm' => [0, 0, 26, 21, 21, 17, 17],
        'f' => [6, 9, 8, 28, 8, 8, 8],
        'i' => [4, 0, 12, 4, 4, 4, 14],
        'l' => [12, 4, 4, 4, 4, 4, 14],
        'c' => [0, 0, 14, 16, 16, 17, 14],
        'a' => [0, 0, 14, 1, 15, 17, 15],
        'h' => [16, 16, 22, 25, 17, 17, 17],
        _ => [0; 7],
    }
}

/// Text in white on a dark box; `centred` puts its middle at `at`.
fn text(img: &mut RgbImage, at: (f64, f64), s: &str, scale: u32, centred: bool) {
    let (cw, chh) = (6 * scale, 7 * scale);
    let tw = cw * s.chars().count() as u32;
    let (mut x0, mut y0) = (at.0 as i64, at.1 as i64);
    if centred {
        x0 -= tw as i64 / 2;
        y0 -= chh as i64 / 2;
    }
    x0 = x0.clamp(0, (img.width() as i64 - tw as i64 - 4).max(0));
    y0 = y0.clamp(0, (img.height() as i64 - chh as i64 - 4).max(0));
    let pad = 2 * scale as i64;
    for y in (y0 - pad).max(0)..(y0 + chh as i64 + pad).min(img.height() as i64) {
        for x in (x0 - pad).max(0)..(x0 + tw as i64 + pad).min(img.width() as i64) {
            let p = img.get_pixel(x as u32, y as u32).0;
            img.put_pixel(x as u32, y as u32, Rgb(blend(p, [0, 0, 0], 0.65)));
        }
    }
    for (k, ch) in s.chars().enumerate() {
        let g = glyph(ch);
        for (row, bits) in g.iter().enumerate() {
            for col in 0..5 {
                if bits & (1 << (4 - col)) != 0 {
                    for dy in 0..scale {
                        for dx in 0..scale {
                            let (x, y) = (x0 + (k as u32 * cw + col * scale + dx) as i64, y0 + (row as u32 * scale + dy) as i64);
                            if x >= 0 && y >= 0 && (x as u32) < img.width() && (y as u32) < img.height() {
                                img.put_pixel(x as u32, y as u32, Rgb([255, 255, 255]));
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn homography_round_trips() {
        let src = [[0.0, 0.0], [100.0, 0.0], [100.0, 50.0], [0.0, 50.0]];
        let dst = [[10.0, 20.0], [410.0, 40.0], [400.0, 260.0], [5.0, 230.0]];
        let h = homography(&src, &dst).unwrap();
        for k in 0..4 {
            let (u, v) = apply(&h, src[k][0], src[k][1]);
            assert!((u - dst[k][0]).abs() < 1e-6 && (v - dst[k][1]).abs() < 1e-6);
        }
        let hi = invert(&h).unwrap();
        let (x, y) = apply(&hi, 400.0, 260.0);
        assert!((x - 100.0).abs() < 1e-6 && (y - 50.0).abs() < 1e-6);
    }

    #[test]
    fn camera_straight_down() {
        // 400 mm above mat point (120, 90), f = 3000 px, centre (2000, 1500).
        let (f, z, x0, y0) = (3000.0, 400.0, 120.0, 90.0);
        let h = [f / z, 0.0, 2000.0 - f * x0 / z, 0.0, f / z, 1500.0 - f * y0 / z, 0.0, 0.0, 1.0];
        let (under, height) = camera(&h, f, 2000.0, 1500.0);
        assert!((height - z).abs() < 1e-6, "{height}");
        assert!((under[0] - x0).abs() < 1e-6 && (under[1] - y0).abs() < 1e-6, "{under:?}");
    }

    #[test]
    fn taps_in_any_order() {
        // A landscape mat seen as is: TL (0,0) TR (300,0) BR (300,200) BL (0,200).
        let want = [(0.0, 0.0), (300.0, 0.0), (300.0, 200.0), (0.0, 200.0)];
        for taps in [[want[2], want[0], want[3], want[1]], [want[0], want[3], want[2], want[1]]] {
            let o = order_taps(taps);
            // Long edges first, going round clockwise: some rotation of `want` by 0 or 2.
            assert!(o == want || o == [want[2], want[3], want[0], want[1]], "{o:?}");
        }
    }

    #[test]
    fn exif_focal_length_is_read() {
        // Big-endian TIFF: IFD0 with an Exif pointer, Exif IFD with FocalLengthIn35mmFilm = 48.
        let mut t = b"MM\0*\0\0\0\x08".to_vec();
        t.extend_from_slice(b"\0\x01\x87\x69\0\x04\0\0\0\x01\0\0\0\x1a\0\0\0\0");
        t.extend_from_slice(b"\0\x01\xa4\x05\0\x03\0\0\0\x01\0\x30\0\0\0\0\0\0");
        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend(t);
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE1];
        jpeg.extend_from_slice(&((app1.len() + 2) as u16).to_be_bytes());
        jpeg.extend(app1);
        jpeg.extend_from_slice(&[0xFF, 0xD9]);
        assert_eq!(focal_35(&jpeg), Some(48.0));
    }

    /// A synthetic shot: the teal mat drawn straight down with a 30 × 20 mm block 6 mm tall on it,
    /// seen from 350 mm up. The fit should find the block's footprint, not its enlarged top.
    #[test]
    fn measures_a_block_on_the_mat() {
        let mat = mat("teal").unwrap();
        let (f, z, cam) = (2200.0, 350.0, [140.0, 108.0]);
        let (w, h) = (2000u32, 1500u32);
        let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
        // Photo px → mat point at height t.
        let back = |u: f64, v: f64, t: f64| (cam[0] + (u - cx) * (z - t) / f, cam[1] + (v - cy) * (z - t) / f);
        let field_rgb = mat.printed_rgb.unwrap();
        let [fx, fy, fw, fh] = mat.field;
        let (bx, by, bw, bd, bt) = (100.0, 80.0, 30.0, 20.0, 6.0);
        let img = RgbImage::from_fn(w, h, |u, v| {
            let (u, v) = (u as f64 + 0.5, v as f64 + 0.5);
            // The block's top first (nearer), then its sides, then the mat.
            let (tx, ty) = back(u, v, bt);
            if tx >= bx && tx <= bx + bw && ty >= by && ty <= by + bd {
                return Rgb([200, 160, 110]);
            }
            let (x, y) = back(u, v, 0.0);
            // Sides: the ray passes through the block's volume below its top.
            for k in 1..12 {
                let (sx, sy) = back(u, v, bt * k as f64 / 12.0);
                if sx >= bx && sx <= bx + bw && sy >= by && sy <= by + bd {
                    return Rgb([120, 90, 60]);
                }
            }
            if mat.markers.iter().any(|m| (x - m.centre[0]).abs() < m.size / 2.0 && (y - m.centre[1]).abs() < m.size / 2.0) {
                let (i, j) = (((x - m_c(&mat, x, y).0) / 4.0).floor() as i64, ((y - m_c(&mat, x, y).1) / 4.0).floor() as i64);
                let border = (x - m_c(&mat, x, y).0).abs() > 6.0 || (y - m_c(&mat, x, y).1).abs() > 6.0;
                return if border || (i + j) % 2 == 0 { Rgb([10, 10, 10]) } else { Rgb([250, 250, 250]) };
            }
            if x >= fx && x <= fx + fw && y >= fy && y <= fy + fh {
                return Rgb(field_rgb);
            }
            Rgb([245, 245, 245])
        });
        let dir = std::env::temp_dir().join(format!("bid-scan-test-{}", crate::now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let photo = dir.join("p.png");
        img.save(&photo).unwrap();
        // Taps a little off each corner marker's centre, in a muddled order.
        let fwd = |x: f64, y: f64| ((cx + (x - cam[0]) * f / z) / w as f64, (cy + (y - cam[1]) * f / z) / h as f64);
        let c = mat.corners();
        let tap = |k: usize, d: f64| {
            let (u, v) = fwd(c[k].centre[0] + d, c[k].centre[1] - d);
            [u, v]
        };
        let focal_35 = f / ((w as f64).hypot(h as f64)) * FRAME_DIAGONAL;
        // Tapped, and found without taps.
        for taps in [Some([tap(2, 2.0), tap(0, -2.5), tap(3, 1.5), tap(1, 2.0)]), None] {
            let input = ScanInput {
                photo: &photo,
                mat: &mat,
                taps,
                focal_35: Some(focal_35),
                height: Some(bt),
                print_scale: 1.0,
                overlay: &dir.join("o.jpg"),
            };
            let m = measure(&input).unwrap();
            assert!((m.size[0] - 30.0).abs() < 0.35 && (m.size[1] - 20.0).abs() < 0.35, "{:?} notes {:?}", m.size, m.notes);
            assert!(m.raw[0] > m.size[0], "the raw outline reads larger: {:?}", m.raw);
            assert!((m.camera_height - z).abs() < 5.0, "{}", m.camera_height);
            assert!(m.notes.is_empty(), "{:?}", m.notes);
            eprintln!("tapped {} size {:?} raw {:?} fill {} cam {} overlay {}", taps.is_some(), m.size, m.raw, m.fill, m.camera_height, dir.join("o.jpg").display());
        }
        if std::env::var_os("BID_KEEP_SCAN_TEST").is_none() {
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// A real photo, for trying the method: `BID_SCAN_PHOTO=… BID_SCAN_TAPS='[[x,y]×4]'
    /// [BID_SCAN_MAT=teal] [BID_SCAN_HEIGHT=mm] [BID_SCAN_SCALE=1] cargo test --release real_photo -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_photo() {
        let env = |k: &str| std::env::var(k).ok();
        let photo_in = PathBuf::from(env("BID_SCAN_PHOTO").expect("BID_SCAN_PHOTO"));
        let bytes = std::fs::read(&photo_in).unwrap();
        let taps: Option<[[f64; 2]; 4]> = env("BID_SCAN_TAPS").map(|t| serde_json::from_str(&t).unwrap());
        let dir = std::env::temp_dir().join(format!("bid-scan-real-{}", crate::now_ms()));
        let games = dir.join("games");
        let file = crate::media::save_photo_bytes(&games, "g", "c", &bytes).unwrap();
        let mat = mat(&env("BID_SCAN_MAT").unwrap_or("teal".into())).unwrap();
        let overlay = dir.join("overlay.jpg");
        let input = ScanInput {
            photo: &crate::media::photo_path(&games, "g", &file),
            mat: &mat,
            taps,
            focal_35: env("BID_SCAN_F35").map(|v| v.parse().unwrap()).or(focal_35(&bytes)),
            height: env("BID_SCAN_HEIGHT").map(|v| v.parse().unwrap()),
            print_scale: env("BID_SCAN_SCALE").map_or(1.0, |v| v.parse().unwrap()),
            overlay: &overlay,
        };
        let m = measure(&input).unwrap();
        eprintln!("RESULT f35 {:?} size {:?} raw {:?} fill {} cam {} corners {:?} notes {:?} overlay {}", focal_35(&bytes), m.size, m.raw, m.fill, m.camera_height, m.corners, m.notes, overlay.display());
    }

    fn m_c(mat: &Mat, x: f64, y: f64) -> (f64, f64) {
        let m = mat.markers.iter().min_by(|a, b| ((a.centre[0] - x).hypot(a.centre[1] - y)).total_cmp(&(b.centre[0] - x).hypot(b.centre[1] - y))).unwrap();
        (m.centre[0], m.centre[1])
    }
}
