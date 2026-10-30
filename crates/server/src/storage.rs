//! From what was measured to how it's stored: `store_as` turns a component's measurements into
//! the packer's fields for a storage kind, and says which measurement each came from. When a
//! kind needs something that wasn't measured, the reason names it.

use serde_json::{Map, Value, json};

use crate::Reason;
use crate::spec::{Component, Kind, MeasureOf, Outline};

pub const KINDS: [&str; 7] = ["cards", "tokens", "mini", "bin", "as_is", "undecided", "box"];

fn r1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// [short, long] of a footprint.
fn footprint(d: [f64; 3]) -> [f64; 2] {
    [d[0].min(d[1]), d[0].max(d[1])]
}

/// The packer's fields for storing `c` as `kind`, from its measurements, with `set` (explicit
/// values) winning. Returns the kind and a line per derived field saying where it came from.
pub fn derive(c: &Component, kind: &str, set: &Map<String, Value>) -> Result<(Kind, Vec<String>), Reason> {
    if !KINDS.contains(&kind) {
        return Err(Reason::new("unknown_kind", format!("stored as one of: {}", KINDS.join(", "))).subject(&c.id));
    }
    let m = c.measure.clone().unwrap_or_default();
    let missing = |what: &str| Reason::new("needs_measure", format!("to store '{}' as {kind}, {what}", c.id)).subject(&c.id);
    let mut v = Map::new();
    let mut why = vec![];
    let mut put = |k: &str, val: Value, from: String| {
        if !set.contains_key(k) {
            why.push(format!("{k} = {val}: {from}"));
            v.insert(k.into(), val);
        }
    };
    // Pieces in the game: the count, else the measured stack's count.
    let count = m.count.or(m.stack_count);
    let count_from = if m.count.is_some() { "the count in the game" } else { "the measured stack's count" };
    // One piece's thickness: measured, or the stack divided by its count.
    let thick = match (m.piece, m.stack, m.stack_count) {
        (Some(p), _, _) => Some((p[2], format!("measured piece, {} mm", p[2]))),
        (None, Some(s), Some(n)) => Some((s[2] / n as f64, format!("measured stack {} mm ÷ {n}", s[2]))),
        _ => None,
    };
    // The footprint from whichever was measured first.
    let piece = m.piece.map(|p| (footprint(p), "the measured piece"));
    let stack = m.stack.map(|s| (footprint(s), "the measured stack"));
    let (fp, fp_from) = match m.of {
        MeasureOf::Piece => piece.or(stack),
        MeasureOf::Stack => stack.or(piece),
    }
    .map_or((None, ""), |(f, w)| (Some(f), w));
    let size_from = |fp: [f64; 2]| format!("{} × {} mm, from {fp_from}", fp[0], fp[1]);
    let derived = (|| -> Result<(), Reason> {
        match kind {
            "cards" => {
                let fp = fp.ok_or_else(|| missing("measure a card or the stack"))?;
                put("size", json!(fp), size_from(fp));
                match (m.stack, m.stack_count, count) {
                    (Some(s), Some(n), Some(total)) if total != n => {
                        let h = r1(s[2] * total as f64 / n as f64);
                        put("stack", json!(h), format!("measured stack {} mm of {n}, scaled to {total} cards", s[2]));
                    }
                    (Some(s), _, _) => put("stack", json!(s[2]), format!("measured stack {} mm", s[2])),
                    (None, _, Some(n)) => {
                        if let Some((t, from)) = thick.clone().filter(|(t, _)| *t >= 0.2) {
                            put("stack", json!(r1(t * n as f64)), format!("{n} cards × {t} mm ({from})"));
                        }
                    }
                    (None, _, None) => return Err(missing("measure the stack, or give the count")),
                }
                if let Some(n) = count {
                    put("count", json!(n), count_from.to_string());
                }
            }
            "tokens" => {
                let fp = fp.ok_or_else(|| missing("measure a token"))?;
                let shape = match m.shape {
                    Some(Outline::Round) => "round",
                    Some(Outline::Hex) => "hex",
                    Some(Outline::Square) => "square",
                    Some(Outline::Rect) => "rect",
                    Some(Outline::Irregular) => "rect",
                    None if (fp[0] - fp[1]).abs() < 0.5 => "square",
                    None => "rect",
                };
                put("shape", json!(shape), match m.shape {
                    Some(Outline::Irregular) => "an irregular outline, held in its bounding rectangle".into(),
                    Some(_) => "the measured outline".into(),
                    None => "no outline given; from the footprint".into(),
                });
                let size = if shape == "round" { json!([fp[1]]) } else { json!(fp) };
                put("size", size, size_from(fp));
                let (t, from) = thick.ok_or_else(|| missing("measure one token's thickness, or a stack and its count"))?;
                put("thickness", json!(r1(t * 100.0) / 100.0), from);
                put("count", json!(count.ok_or_else(|| missing("give the count"))?), count_from.to_string());
            }
            "mini" => {
                let p = m.piece.ok_or_else(|| missing("measure one mini (width, depth, height)"))?;
                let base = p[0].max(p[1]);
                put("base", json!(base), format!("the wider of the mini's {} × {} mm footprint", p[0], p[1]));
                put("height", json!(p[2]), "the measured mini".into());
                put("count", json!(count.unwrap_or(1)), if count.is_some() { count_from.into() } else { "one, no count given".into() });
            }
            "box" => {
                // The measurement is the outside; the packer needs the inside.
                if !set.contains_key("inner") && !set.get("printed").is_some_and(|v| v.is_object()) {
                    return Err(missing("give its inside: inner [length, width, height] mm"));
                }
            }
            "bin" => {
                let s = m.stack.ok_or_else(|| missing("measure the pile they make (width, depth, height)"))?;
                let fp = [s[0].max(s[1]), s[0].min(s[1])];
                put("size", json!(fp), format!("the measured pile, {} × {} mm", fp[0], fp[1]));
                put("depth", json!(s[2]), format!("the measured pile, {} mm high", s[2]));
            }
            _ => {}
        }
        Ok(())
    })();
    if let Err(e) = derived {
        // Explicit values may cover what wasn't measured; if not, say what's missing.
        let mut only = set.clone();
        only.insert("kind".into(), json!(kind));
        return serde_json::from_value::<Kind>(Value::Object(only)).map(|k| (k, vec!["as given".into()])).map_err(|_| e);
    }
    for (k, val) in set {
        v.insert(k.clone(), val.clone());
    }
    v.insert("kind".into(), json!(kind));
    let k: Kind = serde_json::from_value(Value::Object(v)).map_err(|e| Reason::new("bad_field", format!("{kind}: {e}")).subject(&c.id))?;
    Ok((k, why))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{Measure, Source};

    fn comp(m: Measure) -> Component {
        Component { measure: Some(m), ..Component::new("x", "X", Source::Measured, Kind::Undecided) }
    }

    #[test]
    fn a_measured_card_stack_scales_to_the_whole_deck() {
        let c = comp(Measure { of: MeasureOf::Stack, stack: Some([63.5, 88.0, 12.0]), stack_count: Some(40), count: Some(120), ..Default::default() });
        let (k, _) = derive(&c, "cards", &Map::new()).unwrap();
        match k {
            Kind::Cards { size, stack, count, .. } => {
                assert_eq!(size, [63.5, 88.0]);
                assert_eq!(stack, Some(36.0));
                assert_eq!(count, Some(120));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn tokens_take_thickness_from_a_stack() {
        let c = comp(Measure { of: MeasureOf::Piece, piece: Some([25.0, 25.0, 0.0001]), ..Default::default() });
        assert!(derive(&c, "tokens", &Map::new()).is_err(), "no count");
        let c = comp(Measure {
            of: MeasureOf::Stack,
            stack: Some([25.0, 25.0, 20.0]),
            stack_count: Some(10),
            count: Some(30),
            shape: Some(Outline::Round),
            ..Default::default()
        });
        match derive(&c, "tokens", &Map::new()).unwrap().0 {
            Kind::Tokens { size, thickness, count, .. } => assert_eq!((size, thickness, count), (vec![25.0], 2.0, 30)),
            _ => panic!(),
        }
    }

    #[test]
    fn a_bin_needs_the_pile_and_names_it() {
        let c = comp(Measure { of: MeasureOf::Piece, piece: Some([16.0, 16.0, 16.0]), count: Some(12), ..Default::default() });
        let e = derive(&c, "bin", &Map::new()).unwrap_err();
        assert_eq!(e.code, "needs_measure");
        assert!(e.message.contains("pile"));
        let mut set = Map::new();
        set.insert("size".into(), json!([60.0, 40.0]));
        set.insert("depth".into(), json!(20.0));
        assert!(derive(&c, "bin", &set).is_ok(), "explicit values cover what wasn't measured");
    }
}
