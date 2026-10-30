//! Component photos and the face textures cropped from them.
//!
//! Photos are inputs and live with the spec (`games/<slug>/photos/`, in git). A face is four
//! corners marked on a photo, stored in the spec; its texture (`games/<slug>/faces/`, generated,
//! gitignored) is that quad straightened to the component's measured proportions.

use std::path::{Path, PathBuf};

use base64::Engine;
use image::{ImageFormat, RgbImage};

use crate::Reason;
use crate::spec::{Component, Face, Kind, Spec};

/// Photos are kept at most this many pixels on the long side.
const PHOTO_MAX: u32 = 2560;
/// Face textures are this many pixels on the long side.
const FACE_PX: u32 = 1024;

fn io(e: impl std::fmt::Display) -> Reason {
    Reason::new("io", e.to_string())
}

pub fn photo_path(games: &Path, slug: &str, file: &str) -> PathBuf {
    games.join(slug).join("photos").join(file)
}

pub fn face_path(games: &Path, slug: &str, component: &str, face: &str) -> PathBuf {
    games.join(slug).join("faces").join(format!("{component}-{face}.jpg"))
}

/// The faces a component kind can show.
pub fn faces_for(c: &Component) -> &'static [&'static str] {
    match c.kind {
        Kind::Mini { .. } => &["front", "top"],
        // The lid's art, and the box's long and short sides.
        Kind::Box(_) => &["lid", "front", "side"],
        _ => &["top"],
    }
}

/// Decode an uploaded photo (a data URL or bare base64; JPEG or PNG), shrink it to PHOTO_MAX,
/// and store it as JPEG. Returns the file name.
pub fn save_photo(games: &Path, slug: &str, component: &str, data: &str) -> Result<String, Reason> {
    let b64 = data.split_once(";base64,").map_or(data, |(_, b)| b).trim();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| Reason::new("bad_photo", format!("not base64: {e}")))?;
    // Decoded and written afresh, so nothing of the original file's metadata (camera, place,
    // time) is kept; its EXIF orientation is applied first, so a phone photo stays upright.
    let bad = |e: image::ImageError| Reason::new("bad_photo", format!("not a JPEG or PNG: {e}"));
    let mut dec = image::ImageReader::new(std::io::Cursor::new(&bytes)).with_guessed_format().map_err(io)?.into_decoder().map_err(bad)?;
    let orientation = image::ImageDecoder::orientation(&mut dec).unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = image::DynamicImage::from_decoder(dec).map_err(bad)?;
    img.apply_orientation(orientation);
    let img = if img.width().max(img.height()) > PHOTO_MAX { img.resize(PHOTO_MAX, PHOTO_MAX, image::imageops::FilterType::Triangle) } else { img };
    let file = format!("{component}-{}.jpg", crate::now_ms());
    let path = photo_path(games, slug, &file);
    std::fs::create_dir_all(path.parent().unwrap()).map_err(io)?;
    img.to_rgb8().save_with_format(&path, ImageFormat::Jpeg).map_err(io)?;
    Ok(file)
}

/// Long ÷ short side of what the face shows, from the measurements (None: take it from the quad).
/// One of several measured pieces with a regular outline (tokens loose in a bin): the viewer
/// draws each of them in the bin, so the face is cropped to one piece.
pub fn loose_piece(c: &Component) -> Option<[f64; 3]> {
    let m = c.measure.as_ref()?;
    let regular = !matches!(m.shape, Some(crate::spec::Outline::Irregular));
    m.piece.filter(|_| regular && m.count.unwrap_or(1) > 1)
}

fn measured_aspect(c: &Component, face: &str) -> Option<f64> {
    let r = |a: f64, b: f64| a.max(b) / a.min(b);
    match (&c.kind, face) {
        (Kind::Cards { size, .. }, _) => Some(r(size[0], size[1])),
        (Kind::Tokens { size, .. }, _) => Some(r(size[0], *size.get(1).unwrap_or(&size[0]))),
        // A bin of measured loose pieces (tokens) is shown as the pieces: the face is one piece.
        (Kind::Bin { .. }, _) if loose_piece(c).is_some() => loose_piece(c).map(|p| r(p[0], p[1])),
        (Kind::Bin { size, .. }, _) => Some(r(size[0], size[1])),
        (Kind::Mini { .. }, "top") => Some(1.0),
        (Kind::Mini { .. }, _) => None,
        // Not stored yet: the measured footprint.
        // A box: the lid's top, and its sides (front long, side short) as deep as the lid
        // telescopes. Sizes from the measured outside, else the inside.
        (Kind::Box(b), face) => {
            let d = c.measure.as_ref().and_then(|m| m.piece.or(m.stack)).unwrap_or(b.inner);
            let (long, short) = (d[0].max(d[1]), d[0].min(d[1]));
            let skirt = b.lid_height.unwrap_or(d[2]).min(d[2]);
            Some(match face {
                "front" => r(long, skirt),
                "side" => r(short, skirt),
                _ => r(long, short),
            })
        }
        (Kind::AsIs | Kind::Undecided, _) => {
            let m = c.measure.as_ref()?;
            let d = m.piece.or(m.stack)?;
            Some(r(d[0], d[1]))
        }
    }
}

/// Projective map from the unit square to a quad (corners for (0,0), (1,0), (1,1), (0,1)).
fn square_to_quad(q: [[f64; 2]; 4]) -> impl Fn(f64, f64) -> (f64, f64) {
    let [[x0, y0], [x1, y1], [x2, y2], [x3, y3]] = q;
    let (dx1, dx2, dx3) = (x1 - x2, x3 - x2, x0 - x1 + x2 - x3);
    let (dy1, dy2, dy3) = (y1 - y2, y3 - y2, y0 - y1 + y2 - y3);
    let (g, h) = if dx3.abs() < 1e-12 && dy3.abs() < 1e-12 {
        (0.0, 0.0)
    } else {
        let det = dx1 * dy2 - dx2 * dy1;
        ((dx3 * dy2 - dx2 * dy3) / det, (dx1 * dy3 - dx3 * dy1) / det)
    };
    let (a, b, c) = (x1 - x0 + g * x1, x3 - x0 + h * x3, x0);
    let (d, e, f) = (y1 - y0 + g * y1, y3 - y0 + h * y3, y0);
    move |u, v| {
        let w = g * u + h * v + 1.0;
        ((a * u + b * v + c) / w, (d * u + e * v + f) / w)
    }
}

fn sample(img: &RgbImage, x: f64, y: f64) -> [u8; 3] {
    let (w, h) = (img.width() as f64, img.height() as f64);
    let x = x.clamp(0.0, w - 1.001);
    let y = y.clamp(0.0, h - 1.001);
    let (x0, y0) = (x.floor() as u32, y.floor() as u32);
    let (fx, fy) = (x - x0 as f64, y - y0 as f64);
    let p = |dx, dy| img.get_pixel(x0 + dx, y0 + dy).0;
    let (a, b, c, d) = (p(0, 0), p(1, 0), p(0, 1), p(1, 1));
    std::array::from_fn(|i| {
        let top = a[i] as f64 * (1.0 - fx) + b[i] as f64 * fx;
        let bot = c[i] as f64 * (1.0 - fx) + d[i] as f64 * fx;
        (top * (1.0 - fy) + bot * fy).round() as u8
    })
}

/// Straighten a face out of its photo and write the texture.
pub fn render_face(games: &Path, slug: &str, c: &Component, face: &str, f: &Face) -> Result<PathBuf, Reason> {
    let src = image::open(photo_path(games, slug, &f.photo)).map_err(|e| Reason::new("bad_photo", format!("{}: {e}", f.photo)))?.to_rgb8();
    let (pw, ph) = (src.width() as f64, src.height() as f64);
    let q = f.corners.map(|[x, y]| [x * pw, y * ph]);
    let dist = |a: [f64; 2], b: [f64; 2]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
    let across = (dist(q[0], q[1]) + dist(q[3], q[2])) / 2.0;
    let down = (dist(q[0], q[3]) + dist(q[1], q[2])) / 2.0;
    // The measured proportions, in whichever orientation the face was marked; the quad's own
    // proportions when nothing measured gives them (a mini seen from the front).
    let ratio = measured_aspect(c, face).unwrap_or_else(|| across.max(down) / across.min(down).max(1.0));
    let (w, h) = if across >= down {
        (FACE_PX, ((FACE_PX as f64) / ratio).round().max(1.0) as u32)
    } else {
        (((FACE_PX as f64) / ratio).round().max(1.0) as u32, FACE_PX)
    };
    let map = square_to_quad(q);
    let out = RgbImage::from_fn(w, h, |x, y| {
        let (sx, sy) = map((x as f64 + 0.5) / w as f64, (y as f64 + 0.5) / h as f64);
        image::Rgb(sample(&src, sx, sy))
    });
    let path = face_path(games, slug, &c.id, face);
    std::fs::create_dir_all(path.parent().unwrap()).map_err(io)?;
    out.save_with_format(&path, ImageFormat::Jpeg).map_err(io)?;
    Ok(path)
}

/// Make sure every face in the spec has its texture (they are generated, so may be missing).
pub fn ensure_faces(games: &Path, slug: &str, s: &Spec) {
    for c in &s.components {
        for (face, f) in &c.faces {
            let tex = face_path(games, slug, &c.id, face);
            let photo = photo_path(games, slug, &f.photo);
            let stale = match (std::fs::metadata(&tex).and_then(|m| m.modified()), std::fs::metadata(&photo).and_then(|m| m.modified())) {
                (Ok(t), Ok(p)) => t < p,
                _ => true,
            };
            if stale {
                if let Err(e) = render_face(games, slug, c, face, f) {
                    eprintln!("{slug}: face {}/{face}: {}", c.id, e.message);
                }
            }
        }
    }
}

/// A cache key for a face texture URL, so browsers refetch when the crop changes.
pub fn face_version(f: &Face) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    f.photo.hash(&mut h);
    for c in f.corners.iter().flatten() {
        c.to_bits().hash(&mut h);
    }
    format!("{:x}", h.finish())
}

#[cfg(test)]
mod tests {
    use super::{photo_path, save_photo, square_to_quad};
    use base64::Engine;
    use image::ImageFormat;

    #[test]
    fn square_maps_onto_the_quad_corners() {
        let q = [[10.0, 20.0], [110.0, 30.0], [100.0, 160.0], [5.0, 140.0]];
        let m = square_to_quad(q);
        for ((u, v), want) in [((0.0, 0.0), q[0]), ((1.0, 0.0), q[1]), ((1.0, 1.0), q[2]), ((0.0, 1.0), q[3])] {
            let (x, y) = m(u, v);
            assert!((x - want[0]).abs() < 1e-9 && (y - want[1]).abs() < 1e-9, "({u},{v}) -> ({x},{y}), want {want:?}");
        }
    }

    /// A phone photo as `add_photo file=` sends it: stored upright, with no metadata left.
    #[test]
    fn photo_is_turned_upright_and_stripped() {
        let mut jpeg = vec![];
        image::RgbImage::from_pixel(40, 20, image::Rgb([200, 30, 30])).write_to(&mut std::io::Cursor::new(&mut jpeg), ImageFormat::Jpeg).unwrap();
        // APP1 Exif with orientation 6 (turn 90° clockwise) and a camera model.
        let mut tiff = b"MM\0*\0\0\0\x08\0\x02".to_vec();
        tiff.extend_from_slice(b"\x01\x10\0\x02\0\0\0\x04TST\0"); // Model "TST"
        tiff.extend_from_slice(b"\x01\x12\0\x03\0\0\0\x01\0\x06\0\0"); // Orientation 6
        tiff.extend_from_slice(b"\0\0\0\0");
        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend(tiff);
        let len = (app1.len() + 2) as u16;
        let mut photo = jpeg[..2].to_vec();
        photo.extend_from_slice(&[0xFF, 0xE1]);
        photo.extend_from_slice(&len.to_be_bytes());
        photo.extend(app1);
        photo.extend_from_slice(&jpeg[2..]);

        let games = std::env::temp_dir().join(format!("bid-photo-test-{}", crate::now_ms()));
        let data = base64::engine::general_purpose::STANDARD.encode(&photo);
        let file = save_photo(&games, "g", "c", &data).unwrap();
        let saved = std::fs::read(photo_path(&games, "g", &file)).unwrap();
        let img = image::load_from_memory(&saved).unwrap();
        assert_eq!((img.width(), img.height()), (20, 40), "orientation applied");
        assert!(!saved.windows(4).any(|w| w == b"Exif"), "no Exif kept");
        let _ = std::fs::remove_dir_all(&games);
    }
}
