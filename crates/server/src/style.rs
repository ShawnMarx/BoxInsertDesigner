//! The build style guide (`styles/<name>.toml`): tolerances and wall, divider and floor sizes,
//! each with a normal and a minimum. The packer starts at normal and steps down only to fit.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Reason;

const BUILTIN: &str = include_str!("../../../styles/default.toml");

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Style {
    pub name: String,
    /// Dotted keys that are Claude's proposals, not yet confirmed by the user.
    #[serde(default)]
    pub proposed: Vec<String>,
    pub tolerance: Tolerance,
    pub walls: Walls,
    pub floor: Floor,
    /// Steepest allowed overhang, from horizontal; no print may need supports.
    pub max_overhang_deg: f64,
    pub heavy_cards_mm: f64,
    /// Boxes designed to print, with a slide-on lid. Styles saved before they existed get the
    /// built-in values.
    #[serde(default = "PrintedBox::builtin")]
    pub printed_box: PrintedBox,
    /// Holds for getting things out. Styles saved before they existed get the built-in values.
    #[serde(default = "Grips::builtin")]
    pub grips: Grips,
    /// Stencil labels. Styles saved before they existed get the built-in values.
    #[serde(default = "Labels::builtin")]
    pub labels: Labels,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Labels {
    /// How deep the text is cut into its face.
    pub depth: f64,
    /// Capital height: the most a label is given, and the least before it is left off.
    pub height: f64,
    pub min_height: f64,
    /// Clear of every edge, pocket, hole and window around the text.
    pub margin: f64,
}

impl Labels {
    fn builtin() -> Labels {
        Style::builtin().labels
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Grips {
    /// Diameter of a half-circle notch through a tray's side.
    pub half_circle: f64,
    /// A holder's 45° lip: its length, and how far it reaches out (its underside drops as far).
    pub lip_length: f64,
    pub lip_depth: f64,
}

impl Grips {
    fn builtin() -> Grips {
        Style::builtin().grips
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PrintedBox {
    pub wall: f64,
    pub floor: f64,
    /// Depth of the lid's dovetail groove in each wall; the lid is twice this thick.
    pub lid_groove: f64,
    /// Lid to groove, per side.
    pub slide_gap: f64,
    /// Height of the click ridge under the lid, and of its notch in the box.
    pub detent: f64,
}

impl PrintedBox {
    fn builtin() -> PrintedBox {
        Style::builtin().printed_box
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Tolerance {
    pub print_to_component: f64,
    pub print_to_print: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Walls {
    pub normal: f64,
    pub min: f64,
    pub absolute_min: f64,
    pub max: f64,
    /// A tray's four outer vertical corners are rounded to this (0: square). Styles saved before
    /// it existed get the built-in value.
    #[serde(default = "Walls::builtin_corner_radius")]
    pub corner_radius: f64,
}

impl Walls {
    fn builtin_corner_radius() -> f64 {
        2.0
    }

    /// The corner radius for a tray with `wall`: the style's, unless that would leave less than
    /// `absolute_min` between the rounded corner and a pocket's square corner inside it. A radius
    /// r over a wall w leaves r(1 − √2) + w√2 there: 2 mm over 1.6 leaves 1.43.
    pub fn corner_radius_for(&self, wall: f64) -> f64 {
        let s2 = std::f64::consts::SQRT_2;
        let most = if self.corner_radius > wall { (wall * s2 - self.absolute_min) / (s2 - 1.0) } else { wall };
        (self.corner_radius.min(most.max(wall.min(self.corner_radius))) * 100.0).floor() / 100.0
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Floor {
    pub normal: f64,
    pub min: f64,
}

impl Style {
    pub fn builtin() -> Style {
        toml::from_str(BUILTIN).expect("styles/default.toml is valid")
    }

    pub fn validate(&self) -> Result<(), Reason> {
        let w = &self.walls;
        let bad = |m: &str| Err(Reason::new("bad_style", m.to_string()).subject(&self.name));
        if !(w.absolute_min > 0.0 && w.absolute_min <= w.min && w.min <= w.normal && w.normal <= w.max) {
            return bad("walls must satisfy 0 < absolute_min ≤ min ≤ normal ≤ max");
        }
        if !(self.floor.min > 0.0 && self.floor.min <= self.floor.normal) {
            return bad("floor must satisfy 0 < min ≤ normal");
        }
        if self.tolerance.print_to_component < 0.0 || self.tolerance.print_to_print < 0.0 {
            return bad("tolerances can't be negative");
        }
        if !(0.0..=90.0).contains(&self.max_overhang_deg) {
            return bad("max_overhang_deg is 0–90");
        }
        if self.heavy_cards_mm <= 0.0 {
            return bad("heavy_cards_mm must be positive");
        }
        let pb = &self.printed_box;
        if !(pb.lid_groove > 0.0 && pb.floor > 0.0 && pb.detent > 0.0) {
            return bad("printed_box lid_groove, floor and detent must be positive");
        }
        if pb.wall < pb.lid_groove + w.min - 1e-9 {
            return bad("printed_box.wall must leave walls.min outside the lid groove (wall ≥ lid_groove + walls.min)");
        }
        if pb.slide_gap < self.tolerance.print_to_print - 1e-9 {
            return bad("printed_box.slide_gap can't be under tolerance.print_to_print");
        }
        if pb.detent >= pb.lid_groove {
            return bad("printed_box.detent must be under lid_groove, or the lid can't ride over it");
        }
        let g = &self.grips;
        if !(g.half_circle > 0.0 && g.lip_length > 0.0 && g.lip_depth > 0.0) {
            return bad("grips half_circle, lip_length and lip_depth must be positive");
        }
        let l = &self.labels;
        if !(l.depth > 0.0 && l.min_height > 0.0 && l.min_height <= l.height && l.margin >= 0.0) {
            return bad("labels: depth positive, 0 < min_height ≤ height, margin not negative");
        }
        if l.depth >= self.floor.min {
            return bad("labels.depth must be under floor.min, or a label on a pocket floor cuts through it");
        }
        Ok(())
    }

    /// " (proposed)" when `key` is still one of Claude's guesses.
    pub fn mark(&self, key: &str) -> &'static str {
        if self.proposed.iter().any(|k| k == key) { " (proposed)" } else { "" }
    }
}

pub fn dir() -> PathBuf {
    PathBuf::from(crate::config::server("BID_STYLES", "styles").unwrap_or_else(|| "styles".into()))
}

pub fn path(name: &str) -> PathBuf {
    dir().join(format!("{name}.toml"))
}

pub fn load(name: &str) -> Result<Style, Reason> {
    let p = path(name);
    let s: Style = match std::fs::read_to_string(&p) {
        Ok(text) => toml::from_str(&text).map_err(|e| Reason::new("bad_style", format!("{}: {e}", p.display())).subject(name))?,
        Err(_) if name == "default" => Style::builtin(),
        Err(e) => return Err(Reason::new("no_style", format!("{}: {e}", p.display())).subject(name)),
    };
    s.validate()?;
    Ok(s)
}

/// Set dotted keys (`walls.normal`, `heavy_cards_mm` …) and save. A key that is set is no
/// longer a proposal.
pub fn set(name: &str, values: &serde_json::Map<String, serde_json::Value>) -> Result<Style, Reason> {
    let style = load(name)?;
    let mut v = toml::Value::try_from(&style).map_err(|e| Reason::new("internal", e.to_string()))?;
    for (key, val) in values {
        let num = val.as_f64().ok_or_else(|| Reason::new("bad_value", format!("{key} must be a number of mm")).subject(key))?;
        let unknown = || Reason::new("unknown_key", format!("no numeric style key {key}")).subject(key);
        let parts: Vec<&str> = key.split('.').collect();
        let (table, leaf) = match parts.as_slice() {
            [leaf] => (v.as_table_mut().ok_or_else(unknown)?, *leaf),
            [section, leaf] => (v.get_mut(*section).and_then(|t| t.as_table_mut()).ok_or_else(unknown)?, *leaf),
            _ => return Err(unknown()),
        };
        match table.get(leaf) {
            Some(toml::Value::Float(_)) | Some(toml::Value::Integer(_)) => {
                table.insert(leaf.to_string(), toml::Value::Float(num));
            }
            _ => return Err(unknown()),
        }
    }
    let mut style: Style = v.try_into().map_err(|e: toml::de::Error| Reason::new("bad_style", e.to_string()))?;
    style.proposed.retain(|k| !values.contains_key(k));
    style.validate()?;
    save(&style, &path(name))?;
    Ok(style)
}

fn save(style: &Style, p: &Path) -> Result<(), Reason> {
    let body = toml::to_string_pretty(style).map_err(|e| Reason::new("internal", e.to_string()))?;
    let header = "# Build style guide: the standards every insert is generated against. The user's values unless\n\
                  # listed in `proposed` (Claude's starting guesses). All in mm. Edit with `bid set_style`.\n\n";
    std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| Reason::new("io", e.to_string()))?;
    std::fs::write(p, format!("{header}{body}")).map_err(|e| Reason::new("io", e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corner_radius_keeps_absolute_min_round_a_pocket_corner() {
        let w = Walls { normal: 2.0, min: 1.6, absolute_min: 1.2, max: 3.0, corner_radius: 2.0 };
        // 2 mm on the usual walls: 1.6 leaves 1.43 at a pocket's corner, which is allowed.
        assert_eq!(w.corner_radius_for(2.0), 2.0);
        assert_eq!(w.corner_radius_for(1.6), 2.0);
        assert_eq!(w.corner_radius_for(3.0), 2.0);
        // A 1.2 wall can't spare any: its corners round to 1.2, leaving 1.2.
        assert_eq!(w.corner_radius_for(1.2), 1.2);
        assert_eq!(Walls { corner_radius: 0.0, ..w }.corner_radius_for(1.6), 0.0);
    }
}
