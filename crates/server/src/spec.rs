//! The game spec: the master copy. One TOML file per game at `games/<slug>/spec.toml`.
//! Everything else (layouts, geometry, meshes) is generated from it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Reason;
use crate::style::{self, Style};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Spec {
    pub game: GameInfo,
    /// Earlier specs kept boxes apart from components, as `[[boxes]]` or one `[box]` + `[layout]`;
    /// `load` turns them into `box` components and the next save writes the new form.
    #[serde(default, rename = "boxes", skip_serializing)]
    pub legacy_boxes: Vec<GameBox>,
    #[serde(default, rename = "box", skip_serializing)]
    pub legacy_box: Option<BoxSpec>,
    #[serde(default, rename = "layout", skip_serializing)]
    pub legacy_layout: Option<LayoutChoice>,
    pub printer: Printer,
    #[serde(default)]
    pub sleeves: Vec<SleeveProfile>,
    #[serde(default)]
    pub groups: Vec<Group>,
    #[serde(default)]
    pub components: Vec<Component>,
    /// The style guide this game is built to (`styles/<name>.toml`).
    #[serde(default = "default_style")]
    pub style: String,
    /// That style, loaded with the spec.
    #[serde(skip, default = "Style::builtin")]
    pub rules: Style,
    /// Where pieces go on the print plates, where the user chose; the rest are auto-packed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plates: Vec<PlatePin>,
    /// Plate names, plate 1 first ("" for none).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plate_names: Vec<String>,
    /// The filament slots, slot 1 first. Empty: one slot of Bambu PLA Matte, black.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filaments: Vec<Filament>,
    /// Pieces brought in as they are (`import_project`): printed and placed, never regenerated.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pieces: Vec<Piece>,
    /// How this game's stencil labels are made (`set_labels`); trays ask for them with tweaks.
    #[serde(default, skip_serializing_if = "LabelSettings::is_default")]
    pub labels: LabelSettings,
}

/// How a stencil label is made. Both cut the text `labels.depth` into the face.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LabelMode {
    /// The cut is filled flush by a separate part, the label, in its own filament: multi-object
    /// colouring, one object of two parts in the Bambu project.
    #[default]
    Inlay,
    /// Just cut: the text is left open, in the tray's filament.
    Cut,
}

impl LabelMode {
    pub fn as_str(self) -> &'static str {
        match self {
            LabelMode::Inlay => "inlay",
            LabelMode::Cut => "cut",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct LabelSettings {
    /// The game's default; a tray's `label_mode` tweak wins.
    #[serde(default)]
    pub mode: LabelMode,
    /// Filament slot (1-based) of inlaid labels; default slot 1, the trays' own, until set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filament: Option<usize>,
}

impl LabelSettings {
    fn is_default(&self) -> bool {
        *self == LabelSettings::default()
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Place {
    pub at: [f64; 3],
    #[serde(default)]
    pub turn: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<usize>,
}

/// A piece imported as it is: its parts' meshes live in `games/<slug>/pieces/<id>/`, in print
/// orientation with the piece's min corner at the origin.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Piece {
    pub id: String,
    pub name: String,
    /// Where it came from: file and object name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Its bounding box as it prints, mm.
    pub size: [f64; 3],
    /// Filament slot of the piece; parts may have their own.
    #[serde(default = "slot_one")]
    pub filament: usize,
    pub parts: Vec<PiecePart>,
    /// The box it goes in (default the first).
    #[serde(default, rename = "box", skip_serializing_if = "Option::is_none")]
    pub box_: Option<String>,
    /// The name it came in with, once it's been renamed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_name: Option<String>,
    /// Changes the user asked for on this piece, for an agent, as on a tray.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<Request>,
    /// Requests marked read and changes asked for directly, as on a tray.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub log: Vec<Request>,
    /// Its compartments, found in its mesh (`detect_pockets`): print-local, as it prints.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pockets: Vec<PiecePocket>,
    /// The ones made from this one print: one, or one per colour ("repeat for each colour"),
    /// each with its own place.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub copies: Vec<PieceCopy>,
    /// Made in SolidCraft (`import_design`): its design, beside the parts, to open again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub design: Option<String>,
}

fn slot_one() -> usize {
    1
}

/// A compartment of an imported piece: its footprint's min corner and size, and its floor.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PiecePocket {
    pub pos: [f64; 2],
    pub size: [f64; 2],
    /// Floor height (the lowest floor, for a scooped one) and depth below the rim, mm.
    pub floor: f64,
    pub depth: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PiecePart {
    pub name: String,
    /// The mesh, in `games/<slug>/pieces/<piece>/`.
    pub file: String,
    /// Its own filament slot; unset prints in the piece's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filament: Option<usize>,
}

/// One copy of a piece, and where it sits.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct PieceCopy {
    /// "Harmony"; unset for a single copy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The filament slot this copy prints in, over the piece's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filament: Option<usize>,
    /// Min corner of its bounding box as it sits (after `turn` and `flip`), mm: in box
    /// coordinates, or in the parent's when `inside` is set. Unset: not placed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<[f64; 3]>,
    /// Degrees about z, from how it prints: 0, 90, 180 or 270.
    #[serde(default)]
    pub turn: u32,
    /// Turned over from how it prints (a lid printed top down).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flip: bool,
    /// Tipped about x from how it prints, before `turn`: 90 stands a piece printed on its side
    /// back up (y goes up). 0, 90, 180 or 270.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub tilt: u32,
    /// Sits inside another piece's copy: `<piece>` or `<piece>/<copy name>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inside: Option<String>,
    /// The guide layer it first shows in, from the bottom (1-based), for the layer slider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<usize>,
}

/// A filament slot: a Bambu Studio filament preset and the colour loaded.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Filament {
    pub preset: String,
    pub color: String,
}

pub fn default_filament() -> Filament {
    Filament { preset: "Bambu PLA Matte @BBL X1C".into(), color: "#000000".into() }
}

impl Spec {
    /// The filament slots in use: the spec's, or the default one.
    pub fn filament_slots(&self) -> Vec<Filament> {
        if self.filaments.is_empty() { vec![default_filament()] } else { self.filaments.clone() }
    }

    /// The inlaid labels' filament slot (1-based), and its colour.
    pub fn label_slot(&self) -> usize {
        self.labels.filament.unwrap_or(1)
    }

    pub fn label_color(&self) -> String {
        self.filament_slots().get(self.label_slot() - 1).map_or("#ffffff".into(), |f| f.color.clone())
    }
}

/// A piece's place on the print plates, as the user set it (`set_plate`). Unset fields are the
/// plate packer's to choose.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct PlatePin {
    /// `<box>/<tray>`, or `<box>/printed-box` and `<box>/printed-lid`.
    pub piece: String,
    /// 1-based.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plate: Option<usize>,
    /// Degrees about z, after any flip: 0, 90, 180 or 270.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotate: Option<u32>,
    /// Upside down from how it prints by default (turned over about x).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flip: Option<bool>,
    /// Exactly where on its plate (min corner, bed mm), rather than where the packer finds room.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pos: Option<[f64; 2]>,
    /// Filament slot (1-based) for the piece; default slot 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filament: Option<usize>,
    /// Filament slots for named parts of the piece (a label, a logo), over the piece's.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub parts: std::collections::BTreeMap<String, usize>,
}

fn default_style() -> String {
    "default".into()
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GameInfo {
    pub name: String,
    /// A made-up game to have something on screen; the UI labels it SAMPLE.
    #[serde(default)]
    pub sample: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// Where a measured number came from. Derived numbers carry their own `why` text instead.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    #[default]
    Measured,
    Estimate,
    Sample,
    Default,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BoxSpec {
    /// Inside of the box base: length (x), width (y), height (z), mm.
    pub inner: [f64; 3],
    /// Extra room inside the closed lid above the base rim, mm. 0 when the lid rests on the rim
    /// or telescopes the full depth (the usual case).
    #[serde(default)]
    pub lid_depth: f64,
    /// What lies on top of the insert (boards, rulebook), mm.
    #[serde(default)]
    pub top_stack: f64,
    /// How far the closed lid may ride above the rim when the insert stands proud of it, mm.
    #[serde(default)]
    pub lid_lift: f64,
    #[serde(default)]
    pub source: Source,
}

/// A box as the packer sees it: built from a `box` component (`Spec::boxes`).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GameBox {
    pub id: String,
    pub name: String,
    #[serde(flatten)]
    pub size: BoxSpec,
    #[serde(default)]
    pub layout: LayoutChoice,
    /// Designed to print: the packer sizes it to its insert (`inner` is unset).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub printed: Option<PrintedBox>,
}

/// A box designed to print instead of bought: sized to what goes in it, with a lid that slides
/// in from one end and clicks shut. Walls, grooves and gaps come from the style's `printed_box`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct PrintedBox {
    /// Largest inside allowed, length × width × height mm (to fit a shelf or another box); unset
    /// means as large as the bed prints. A 0 leaves that side to the bed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_inner: Option<[f64; 3]>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Printer {
    pub name: String,
    pub nozzle: f64,
    /// Usable bed, x × y, mm.
    pub bed: [f64; 2],
    pub max_z: f64,
    /// Kept clear at the bed edges, mm.
    pub bed_margin: f64,
    /// Tray heights are whole layers.
    pub layer_height: f64,
    pub line_width: f64,
    /// Bed areas nothing may print on, [x0, y0, x1, y1] mm (the X1C's: its front-left corner).
    #[serde(default = "x1c_bed_exclude")]
    pub bed_exclude: Vec<[f64; 4]>,
}

/// From Bambu Studio's X1C 0.4 machine settings (`bed_exclude_area`).
fn x1c_bed_exclude() -> Vec<[f64; 4]> {
    vec![[0.0, 0.0, 18.0, 28.0]]
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SleeveProfile {
    pub id: String,
    pub name: String,
    /// Thickness of one sleeved card, mm.
    pub per_card_mm: f64,
    #[serde(default)]
    pub source: Source,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Group {
    pub id: String,
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Component {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default)]
    pub source: Source,
    /// Gap per side between this component and its pocket, when it needs more than the style's
    /// print-to-component minimum.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clearance: Option<f64>,
    /// Photos of the component, in `games/<slug>/photos/`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub photos: Vec<Photo>,
    /// Faces cropped from those photos (`top`, or `front` for a mini); each becomes a texture on
    /// the component's placeholder in the viewer.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub faces: std::collections::BTreeMap<String, Face>,
    /// What was measured, as measured. `store_as` derives the packer's fields (`kind`) from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measure: Option<Measure>,
    /// How it should go in, in the user's words ("a pile", "stacks of 20", "by player").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// What its pocket's floor label says (`pocket_labels`), when not its name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Components used with this one at the table, so they're worth keeping together.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub goes_with: Vec<String>,
    /// The box it goes in; unset means the first box.
    #[serde(default, rename = "box", skip_serializing_if = "Option::is_none")]
    pub box_: Option<String>,
    /// Kept in an imported piece (its id) rather than a pocket the packer makes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub piece: Option<String>,
    /// Which of that piece's pockets it's in (indices into its `pockets`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pockets: Vec<usize>,
    /// Where an as-is component (a board, a rulebook) lies in the box, rather than on top of the
    /// insert: its min corner, mm, a turn about z, and the guide layer it shows in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<Place>,
    /// How it is stored: the packer's input. `undecided` until someone picks.
    #[serde(flatten)]
    pub kind: Kind,
}

/// A component as measured. At least one of `piece` and `stack` is given: whichever `of` says
/// was measured first; the other is optional.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Measure {
    /// Which measurement is the main one.
    #[serde(default)]
    pub of: MeasureOf,
    /// One piece: width, depth, height (thickness), mm.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub piece: Option<[f64; 3]>,
    /// A stack or pile: width, depth, height, mm.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack: Option<[f64; 3]>,
    /// How many pieces were in the measured stack.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack_count: Option<u32>,
    /// How many there are in the game.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<Outline>,
    /// More measurements, line by line: a thinner part, a wider part, the sides of a hex.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra: Vec<ExtraMeasure>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum MeasureOf {
    #[default]
    Piece,
    Stack,
}

/// The outline seen from above.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Outline {
    Rect,
    Square,
    Round,
    Hex,
    Irregular,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ExtraMeasure {
    pub label: String,
    pub mm: f64,
}

impl Component {
    pub fn kind_name(&self) -> &'static str {
        match self.kind {
            Kind::Cards { .. } => "cards",
            Kind::Mini { .. } => "mini",
            Kind::Tokens { .. } => "tokens",
            Kind::Bin { .. } => "bin",
            Kind::AsIs => "as_is",
            Kind::Undecided => "undecided",
            Kind::Box(_) => "box",
        }
    }

    /// Goes in an insert tray (not a box, not as-is, and its storage is decided).
    pub fn packs(&self) -> bool {
        !matches!(self.kind, Kind::AsIs | Kind::Undecided | Kind::Box(_)) && self.piece.is_none()
    }

    /// The box it goes in: its own, or the first. A box goes in none.
    pub fn box_in<'a>(&'a self, s: &'a Spec) -> Option<&'a str> {
        if self.is_box() {
            return None;
        }
        self.box_.as_deref().or_else(|| s.components.iter().find(|c| c.is_box()).map(|b| b.id.as_str()))
    }

    pub fn is_box(&self) -> bool {
        matches!(self.kind, Kind::Box(_))
    }

    /// The box fields of a `box` component (callers check `is_box`).
    pub fn box_kind_mut(&mut self) -> &mut BoxKind {
        match &mut self.kind {
            Kind::Box(b) => b,
            _ => panic!("'{}' is not a box", self.id),
        }
    }

    fn game_box(&self) -> Option<GameBox> {
        match &self.kind {
            Kind::Box(b) => Some(GameBox {
                id: self.id.clone(),
                name: self.name.clone(),
                size: BoxSpec { inner: b.inner, lid_depth: b.lid_depth, top_stack: b.top_stack, lid_lift: b.lid_lift, source: self.source },
                layout: b.layout.clone(),
                printed: b.printed.clone(),
            }),
            _ => None,
        }
    }
}

/// A game box: a base with a telescoping lid, holding an insert. Its outside closed, if
/// measured, is in the component's `measure`. Or a box designed to print (`printed`), with a
/// slide-on lid, whose size the packer works out.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BoxKind {
    /// Inside of the box base: length (x), width (y), height (z), mm. Unset for a printed box.
    #[serde(default, skip_serializing_if = "is_unset")]
    pub inner: [f64; 3],
    /// Extra room inside the closed lid above the base rim, mm. 0 for a telescoping lid whose
    /// top rests on the rim, the usual game box.
    #[serde(default)]
    pub lid_depth: f64,
    /// Outside height of the lid's sides, mm: how far it telescopes down the base. Unset means
    /// the full depth of the box. Only the 3D view and the lid's side photos use it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lid_height: Option<f64>,
    /// Lid lift allowed, mm: a design choice. The insert (and the top stack) may stand up to this
    /// much above the rim, so the closed lid sits that much high. The packer uses only what it
    /// needs, and none when the insert fits under a closed lid.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub lid_lift: f64,
    /// What lies on top of the insert (boards, rulebook), mm.
    #[serde(default)]
    pub top_stack: f64,
    /// The layout chosen for its insert, and per-tray tweaks.
    #[serde(default)]
    pub layout: LayoutChoice,
    /// Designed to print rather than bought: the lid fields don't apply, and `inner` is derived.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub printed: Option<PrintedBox>,
}

fn is_unset(v: &[f64; 3]) -> bool {
    v.iter().all(|x| *x == 0.0)
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Photo {
    pub file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// A face marked on a photo: its corners in photo coordinates (0–1, from the top left), in the
/// order top-left, top-right, bottom-right, bottom-left as the face should read.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Face {
    pub photo: String,
    pub corners: [[f64; 2]; 4],
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Kind {
    /// A deck stored flat. Give a measured `stack`, or `count` (+ `sleeve`) to derive it.
    Cards {
        /// Footprint as the cards sit, sleeved if they are sleeved: [short, long] mm.
        size: [f64; 2],
        #[serde(default, skip_serializing_if = "Option::is_none")]
        count: Option<u32>,
        /// Measured stack height, mm. Wins over count × thickness.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stack: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sleeve: Option<String>,
    },
    /// Standing minis, one round pocket each.
    Mini {
        base: f64,
        height: f64,
        #[serde(default = "one")]
        count: u32,
    },
    /// Flat tokens, stored in stacks.
    Tokens {
        shape: TokenShape,
        /// [w, h] mm; for round tokens the diameter twice (or a one-element array, or a number).
        #[serde(deserialize_with = "one_or_more")]
        size: Vec<f64>,
        thickness: f64,
        count: u32,
    },
    /// A plain open bin for loose bits: inside length × width, and depth, mm.
    Bin { size: [f64; 2], depth: f64 },
    /// Goes in the box without an insert (a board, the rulebook).
    #[serde(rename = "as_is")]
    AsIs,
    /// Measured, but how it's stored isn't decided yet; it gets no pocket.
    Undecided,
    /// A box of the game, which holds an insert of its own.
    Box(BoxKind),
}

/// A number or a list of numbers, as a list.
fn one_or_more<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<f64>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMore {
        One(f64),
        More(Vec<f64>),
    }
    Ok(match OneOrMore::deserialize(d)? {
        OneOrMore::One(v) => vec![v],
        OneOrMore::More(v) => v,
    })
}

fn one() -> u32 {
    1
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TokenShape {
    Round,
    Square,
    Rect,
    /// Held in its bounding rectangle for now.
    Hex,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LayoutChoice {
    pub strategy: String,
    #[serde(default)]
    pub tweaks: Vec<Tweak>,
}

impl Default for LayoutChoice {
    fn default() -> Self {
        LayoutChoice { strategy: "by-group".into(), tweaks: vec![] }
    }
}

/// A per-tray override, keyed by tray id. Trays ids are stable for a strategy and a spec.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Tweak {
    pub tray: String,
    /// Deepest pocket allowed, mm. Stacks deeper than this are split.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<f64>,
    /// Outer walls and dividers for this tray, mm: down to the style's absolute minimum, for
    /// small supported trays only. Otherwise the style decides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub walls: Option<f64>,
    /// Finger holes through the floor under card pockets (default on).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finger_holes: Option<bool>,
    /// Card holders: card pockets lose their short end walls (cut out to the tray's edge) and
    /// most of their floor, so a deck is gripped from either end and pushed up from below
    /// (default off).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_ends: Option<bool>,
    /// A floorless frame with a lift-out holder in each bay (open ends, a floor window), and
    /// finger slots between the bays (default off). Wins over `per_pocket`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holders: Option<bool>,
    /// A frame of holders: each holder gets a 45° lip on its long wall that faces a finger slot,
    /// reaching out into the slot to pull it up by (default off).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grips: Option<bool>,
    /// Grab holes: a pair of half-circles facing each other across the tray, top to bottom,
    /// between pockets: "middle", or a component id for just past that component's pocket.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub half_circle: Option<String>,
    /// A stencil label on the tray's top face: this text, placed and sized by the packer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// A stencil label on each pocket's floor, naming what goes in it (default off).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pocket_labels: Option<bool>,
    /// Inlay or cut, for this tray's labels; default the game's (`labels.mode`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_mode: Option<LabelMode>,
    /// Pockets along the tray in this order (component ids); the rest follow in the packer's.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub order: Vec<String>,
    /// Settings for one pocket, by the component it holds.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub pockets: std::collections::BTreeMap<String, PocketTweak>,
    /// One tray per pocket, so each stack lifts out on its own (default off).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_pocket: Option<bool>,
    /// Rounded pocket floors for scooping bits out (default on for bins and tokens).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scoops: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// A name for the tray, in place of the packer's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Changes wanted for this tray, written for an agent to act on, and how that's going.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<Request>,
    /// Requests marked read and changes asked for directly, oldest first: the tray's story.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub log: Vec<Request>,
    /// The tray prints as a SolidCraft edit the user kept (`keep_edit`), not as generated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit: Option<KeptEdit>,
}

/// A SolidCraft edit kept as the tray: the design is `edits/<tray>.kept.sc`, and the features it
/// added to the generated tray are `edits/<tray>.kept.json`, to put on again when the generated
/// tray changes.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct KeptEdit {
    /// The generated tray it was made on (`geometry::edit_key`): while the tray is still that,
    /// the design prints as it is.
    pub base_key: String,
    pub kept_ms: u64,
    /// Features it added (these are put on again), and generated features it changed (these
    /// hold only while the tray is unchanged).
    pub added: usize,
    pub changed: usize,
}

/// One pocket's own settings (a tray tweak, by component).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct PocketTweak {
    /// A rounded floor to scoop from, over the tray's setting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scoop: Option<bool>,
    /// A slot down through both long side walls at the middle, to reach in and lift out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notch: Option<Notch>,
    /// Takes the room left over in the tray (when it ends where that room starts), rather than
    /// leaving it as an empty spare bin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill_spare: Option<bool>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Notch {
    /// How wide the slot is along the wall, mm.
    pub width: f64,
    /// How much wall it leaves standing above the pocket's floor, mm.
    pub keep: f64,
    /// With `keep` 0: the slot goes on down through the floor and bites this far into it past
    /// the wall as a half-circle (at most half the slot's width), to push a stack up from below.
    /// 0: the floor stays whole.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub floor: f64,
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

/// The tweak "tray" that stands for a box's insert as a whole: notes and their history, no settings.
pub const WHOLE_INSERT: &str = "*";

/// A change the user asked for, for an agent: polled with `requests` / `wait_requests`, answered
/// with `update_request`. Editing the text makes it a new, open request.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Request {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub status: RequestStatus,
    pub created_ms: u64,
    pub updated_ms: u64,
    /// The agent's answer: what it did, or why not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<String>,
    /// Asked directly (in a session, say) rather than left as a note: logged once done.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub direct: bool,
    /// When the user marked the answer read, which took it off the tray into its log.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_ms: Option<u64>,
}

impl Request {
    pub fn new(text: String) -> Request {
        let now = crate::now_ms();
        Request { id: format!("r{now}"), text, status: RequestStatus::Open, created_ms: now, updated_ms: now, reply: None, direct: false, read_ms: None }
    }
}

/// New notes for a tray or piece: changed text is a new request, the same text keeps the old one,
/// and empty removes it. A request already answered goes to the log rather than away.
pub fn set_notes(request: &mut Option<Request>, log: &mut Vec<Request>, text: &str) {
    let v = text.trim();
    if request.as_ref().is_some_and(|r| r.text == v) {
        return;
    }
    if let Some(old) = request.take().filter(|r| !r.status.pending()) {
        log.push(old);
    }
    *request = (!v.is_empty()).then(|| Request::new(v.to_string()));
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum RequestStatus {
    #[default]
    Open,
    Working,
    Done,
    Declined,
}

impl RequestStatus {
    /// Still for an agent to pick up or finish.
    pub fn pending(self) -> bool {
        matches!(self, RequestStatus::Open | RequestStatus::Working)
    }
}

pub fn is_slug(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 48
        && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !s.starts_with('-')
}

fn bad(code: &str, subject: &str, message: String) -> Reason {
    Reason::new(code, message).subject(subject)
}

fn positive(v: f64, what: &str, subject: &str) -> Result<(), Reason> {
    if v.is_finite() && v > 0.0 {
        Ok(())
    } else {
        Err(bad("not_positive", subject, format!("{what} must be a positive number of mm, got {v}")))
    }
}

impl Spec {
    pub fn validate(&self) -> Result<(), Reason> {
        for p in &self.plates {
            if p.rotate.is_some_and(|r| r % 90 != 0 || r >= 360) {
                return Err(bad("bad_rotate", &p.piece, "rotate is 0, 90, 180 or 270".into()));
            }
            if p.plate == Some(0) {
                return Err(bad("bad_plate", &p.piece, "plates are numbered from 1".into()));
            }
            let n = self.filament_slots().len();
            for f in p.filament.iter().chain(p.parts.values()) {
                if *f == 0 || *f > n {
                    return Err(bad("bad_filament", &p.piece, format!("filament slots are 1 to {n}")));
                }
            }
            if p.pos.is_some() && p.plate.is_none() {
                return Err(bad("bad_plate", &p.piece, "a set position needs a plate".into()));
            }
        }
        if let Some(f) = self.labels.filament {
            let n = self.filament_slots().len();
            if f == 0 || f > n {
                return Err(bad("bad_filament", "labels", format!("filament slots are 1 to {n}")));
            }
        }
        for f in &self.filaments {
            if !(f.color.len() == 7 && f.color.starts_with('#') && f.color[1..].chars().all(|x| x.is_ascii_hexdigit())) {
                return Err(bad("bad_color", &f.preset, "a filament colour is #rrggbb".into()));
            }
            if !crate::bambu::presets().contains(&f.preset.as_str()) {
                return Err(bad("bad_preset", &f.preset, format!("filament presets: {}", crate::bambu::presets().join(", "))));
            }
        }
        let boxes = self.boxes();
        let box_ids: std::collections::HashSet<&str> = boxes.iter().map(|b| b.id.as_str()).collect();
        for gb in &boxes {
            let (b, id) = (&gb.size, gb.id.as_str());
            if let Some(pb) = &gb.printed {
                if b.top_stack < 0.0 {
                    return Err(bad("negative", id, "top_stack can't be negative".into()));
                }
                for v in pb.max_inner.iter().flatten() {
                    if !(v.is_finite() && *v >= 0.0) {
                        return Err(bad("negative", id, "max_inner sides are mm, or 0 for what the bed prints".into()));
                    }
                }
                self.validate_tweaks(&gb.layout)?;
                continue;
            }
            for (v, w) in b.inner.iter().zip(["box length", "box width", "box height"]) {
                positive(*v, w, id)?;
            }
            if b.lid_depth < 0.0 || b.top_stack < 0.0 {
                return Err(bad("negative", id, "lid_depth and top_stack can't be negative".into()));
            }
            if b.top_stack >= b.inner[2] + b.lid_depth {
                return Err(bad("no_room", id, "the top stack fills the whole box height".into()));
            }
            if b.lid_lift < 0.0 {
                return Err(bad("negative", id, "lid_lift can't be negative".into()));
            }
            if let Some(Kind::Box(k)) = self.component(id).map(|c| &c.kind) {
                if let Some(h) = k.lid_height {
                    positive(h, "lid height", id)?;
                    if b.lid_lift >= h {
                        return Err(bad("lid_off", id, format!("a {} mm lid lift is the whole {h} mm lid height: the lid would sit off the box", b.lid_lift)));
                    }
                }
            }
            self.validate_tweaks(&gb.layout)?;
        }
        let p = &self.printer;
        for v in [p.nozzle, p.bed[0], p.bed[1], p.max_z, p.layer_height, p.line_width] {
            positive(v, "printer dimension", "printer")?;
        }
        let mut ids = std::collections::HashSet::new();
        for s in &self.sleeves {
            positive(s.per_card_mm, "sleeve per_card_mm", &s.id)?;
        }
        for g in &self.groups {
            if !is_slug(&g.id) || !ids.insert(g.id.clone()) {
                return Err(bad("bad_id", &g.id, format!("group id '{}' must be a unique slug (a-z, 0-9, -)", g.id)));
            }
        }
        for c in &self.components {
            if let Some(l) = &c.label {
                if l.trim().is_empty() || l.chars().count() > 40 || l.contains('\n') {
                    return Err(bad("bad_label", &c.id, "a label is one line of 1 to 40 characters".into()));
                }
            }
            if !is_slug(&c.id) || !ids.insert(c.id.clone()) {
                return Err(bad(
                    "bad_id",
                    &c.id,
                    format!("component id '{}' must be a unique slug (a-z, 0-9, -), not shared with a group", c.id),
                ));
            }
            if let Some(cl) = c.clearance {
                let min = self.rules.tolerance.print_to_component;
                if !(cl + 1e-9 >= min) {
                    return Err(bad("below_tolerance", &c.id, format!("clearance {cl} mm is under the style's {min} mm print-to-component minimum")));
                }
            }
            for (face, f) in &c.faces {
                if !c.photos.iter().any(|p| p.file == f.photo) {
                    return Err(bad("no_photo", &c.id, format!("face '{face}' uses photo '{}', which this component doesn't have", f.photo)));
                }
                if f.corners.iter().flatten().any(|v| !(0.0..=1.0).contains(v)) {
                    return Err(bad("bad_corners", &c.id, "corners are fractions of the photo, 0–1".into()));
                }
            }
            if let Some(g) = &c.group {
                if !self.groups.iter().any(|x| &x.id == g) {
                    return Err(bad("unknown_group", &c.id, format!("group '{g}' doesn't exist; add_group first")));
                }
            }
            if let Some(b) = &c.box_ {
                if c.is_box() {
                    return Err(bad("box_in_box", &c.id, "a box doesn't go inside another box".into()));
                }
                if !box_ids.contains(b.as_str()) {
                    return Err(bad("unknown_box", &c.id, format!("box '{b}' doesn't exist; add_box first")));
                }
            }
            for w in &c.goes_with {
                if w == &c.id || !self.components.iter().any(|x| &x.id == w) {
                    return Err(bad("unknown_component", &c.id, format!("goes_with '{w}' isn't another component")));
                }
            }
            if let Some(m) = &c.measure {
                m.validate(&c.id)?;
            }
            match &c.kind {
                Kind::Cards { size, count, stack, sleeve } => {
                    positive(size[0], "card width", &c.id)?;
                    positive(size[1], "card length", &c.id)?;
                    if let Some(s) = stack {
                        positive(*s, "stack", &c.id)?;
                    }
                    if stack.is_none() && count.is_none() {
                        return Err(bad("needs_stack_or_count", &c.id, "cards need a measured stack (mm) or a count".into()));
                    }
                    if count == &Some(0) {
                        return Err(bad("not_positive", &c.id, "count must be at least 1".into()));
                    }
                    if let Some(sl) = sleeve {
                        if !self.sleeves.iter().any(|x| &x.id == sl) {
                            let known: Vec<_> = self.sleeves.iter().map(|s| s.id.as_str()).collect();
                            return Err(bad("unknown_sleeve", &c.id, format!("sleeve profile '{sl}' doesn't exist (have: {})", known.join(", "))));
                        }
                    }
                }
                Kind::Mini { base, height, count } => {
                    positive(*base, "mini base", &c.id)?;
                    positive(*height, "mini height", &c.id)?;
                    if *count == 0 {
                        return Err(bad("not_positive", &c.id, "count must be at least 1".into()));
                    }
                }
                Kind::Tokens { size, thickness, count, shape } => {
                    if size.is_empty() || size.len() > 2 {
                        return Err(bad("bad_size", &c.id, "token size is [w, h] (or [d] for round)".into()));
                    }
                    for v in size {
                        positive(*v, "token size", &c.id)?;
                    }
                    if *shape == TokenShape::Round && size.len() == 2 && (size[0] - size[1]).abs() > 1e-9 {
                        return Err(bad("bad_size", &c.id, "a round token has one diameter".into()));
                    }
                    positive(*thickness, "token thickness", &c.id)?;
                    if *count == 0 {
                        return Err(bad("not_positive", &c.id, "count must be at least 1".into()));
                    }
                }
                Kind::Bin { size, depth } => {
                    positive(size[0], "bin length", &c.id)?;
                    positive(size[1], "bin width", &c.id)?;
                    positive(*depth, "bin depth", &c.id)?;
                }
                Kind::AsIs | Kind::Undecided | Kind::Box(_) => {}
            }
        }
        Ok(())
    }

    fn validate_tweaks(&self, layout: &LayoutChoice) -> Result<(), Reason> {
        for t in &layout.tweaks {
            if let Some(d) = t.max_depth {
                positive(d, "max_depth", &t.tray)?;
            }
            if let Some(w) = t.walls {
                let wl = &self.rules.walls;
                if w + 1e-9 < wl.absolute_min || w > wl.max + 1e-9 {
                    return Err(bad("out_of_style", &t.tray, format!("walls {w} mm is outside the style's {}–{} mm", wl.absolute_min, wl.max)));
                }
            }
            for (c, pt) in &t.pockets {
                if self.component(c).is_none() {
                    return Err(bad("no_component", &t.tray, format!("pocket settings for '{c}', which isn't a component")));
                }
                if let Some(n) = pt.notch {
                    if !(n.width > 0.0) || n.keep < 0.0 {
                        return Err(bad("bad_notch", &t.tray, "a notch is wider than 0 mm and keeps 0 mm or more of wall".into()));
                    }
                }
            }
            if let Some(o) = t.order.iter().find(|o| self.component(o).is_none()) {
                return Err(bad("no_component", &t.tray, format!("order names '{o}', which isn't a component")));
            }
            if let Some(l) = &t.label {
                if l.trim().is_empty() || l.chars().count() > 40 || l.contains('\n') {
                    return Err(bad("bad_label", &t.tray, "a label is one line of 1 to 40 characters".into()));
                }
            }
        }
        Ok(())
    }

    pub fn component(&self, id: &str) -> Option<&Component> {
        self.components.iter().find(|c| c.id == id)
    }

    /// The boxes, in the order their components are listed.
    pub fn boxes(&self) -> Vec<GameBox> {
        self.components.iter().filter_map(|c| c.game_box()).collect()
    }

    pub fn game_box(&self, id: &str) -> Option<GameBox> {
        self.component(id).and_then(|c| c.game_box())
    }

    /// The box a command means: the one named, or the first. A reason when there's none.
    pub fn pick_box(&self, id: Option<&str>) -> Result<GameBox, Reason> {
        let boxes = self.boxes();
        let ids = || boxes.iter().map(|b| b.id.as_str()).collect::<Vec<_>>().join(", ");
        match id {
            Some(id) => boxes.iter().find(|b| b.id == id).cloned().ok_or_else(|| Reason::new("no_box", format!("no box '{id}' (have: {})", ids())).subject(id)),
            None => boxes.first().cloned().ok_or_else(|| Reason::new("no_box", "this game has no box yet; add_box first")),
        }
    }

    /// The `box` component a command means (see `pick_box`).
    pub fn pick_box_mut(&mut self, id: Option<&str>) -> Result<&mut Component, Reason> {
        let want = self.pick_box(id)?.id;
        Ok(self.components.iter_mut().find(|c| c.id == want).unwrap())
    }

    /// Where a new box goes in the list: after the other boxes.
    pub fn box_slot(&self) -> usize {
        self.components.iter().rposition(|c| c.is_box()).map_or(0, |i| i + 1)
    }

    /// The components the packer places in box `id`.
    pub fn packed_in<'a>(&'a self, id: &'a str) -> impl Iterator<Item = &'a Component> + 'a {
        self.components.iter().filter(move |c| c.packs() && c.box_in(self) == Some(id))
    }

    /// Turn boxes kept apart from components (`[[boxes]]`, or `[box]` + `[layout]`) into `box`
    /// components at the top of the list.
    fn migrate(&mut self) {
        let mut old = std::mem::take(&mut self.legacy_boxes);
        if let Some(b) = self.legacy_box.take() {
            if old.is_empty() {
                let layout = self.legacy_layout.take().unwrap_or_default();
                old.push(GameBox { id: "main".into(), name: "Game box".into(), size: b, layout, printed: None });
            }
        }
        self.legacy_layout = None;
        for (i, gb) in old.into_iter().enumerate() {
            let taken = self.components.iter().any(|c| c.id == gb.id);
            let id = if taken { format!("{}-box", gb.id) } else { gb.id.clone() };
            for c in self.components.iter_mut().filter(|c| c.box_.as_deref() == Some(&gb.id)) {
                c.box_ = Some(id.clone());
            }
            let b = gb.size;
            let kind = Kind::Box(BoxKind { inner: b.inner, lid_depth: b.lid_depth, lid_height: None, lid_lift: b.lid_lift, top_stack: b.top_stack, layout: gb.layout, printed: None });
            self.components.insert(i, Component::new(&id, &gb.name, b.source, kind));
        }
    }
}

impl Component {
    pub fn new(id: &str, name: &str, source: Source, kind: Kind) -> Component {
        Component {
            id: id.into(),
            name: name.into(),
            group: None,
            source,
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
            kind,
        }
    }
}

impl Measure {
    fn validate(&self, id: &str) -> Result<(), Reason> {
        let main = match self.of {
            MeasureOf::Piece => self.piece,
            MeasureOf::Stack => self.stack,
        };
        if main.is_none() {
            return Err(bad("needs_measure", id, format!("measured as a {:?}, so give the {0:?} width, depth and height", self.of).to_lowercase()));
        }
        for v in self.piece.iter().chain(self.stack.iter()).flatten() {
            positive(*v, "measurement", id)?;
        }
        if self.stack_count == Some(0) || self.count == Some(0) {
            return Err(bad("not_positive", id, "counts must be at least 1".into()));
        }
        for e in &self.extra {
            positive(e.mm, &format!("'{}'", e.label), id)?;
        }
        Ok(())
    }
}

pub fn spec_path(games: &Path, slug: &str) -> PathBuf {
    games.join(slug).join("spec.toml")
}

pub fn load(games: &Path, slug: &str) -> Result<Spec, Reason> {
    let path = spec_path(games, slug);
    let text = std::fs::read_to_string(&path)
        .map_err(|e| Reason::new("no_game", format!("can't read {}: {e}", path.display())).subject(slug))?;
    let mut spec: Spec = toml::from_str(&text).map_err(|e| Reason::new("bad_spec", format!("{}: {e}", path.display())).subject(slug))?;
    spec.rules = style::load(&spec.style)?;
    spec.migrate();
    spec.validate()?;
    Ok(spec)
}

const HEADER: &str = "# BoxInsertDesigner game spec: the master copy. Edit through `bid` or the web app;\n\
                      # hand edits are fine too (the server picks them up on the next command).\n\n";

pub fn save(games: &Path, slug: &str, spec: &Spec) -> Result<(), Reason> {
    let path = spec_path(games, slug);
    let text = toml::to_string_pretty(spec).map_err(|e| Reason::new("internal", e.to_string()))?;
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| Reason::new("io", e.to_string()))?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, format!("{HEADER}{text}")).map_err(|e| Reason::new("io", e.to_string()))?;
    std::fs::rename(&tmp, &path).map_err(|e| Reason::new("io", e.to_string()))?;
    Ok(())
}

pub fn list_games(games: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(games)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join("spec.toml").is_file())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    v.sort();
    v
}

/// The X1C with a 0.4 mm nozzle. Walls, floors and tolerances are in the style guide.
pub fn default_printer() -> Printer {
    Printer {
        name: "Bambu Lab X1C, 0.4 mm nozzle".into(),
        nozzle: 0.4,
        bed: [256.0, 256.0],
        max_z: 256.0,
        bed_margin: 3.0,
        layer_height: 0.2,
        line_width: 0.42,
        bed_exclude: x1c_bed_exclude(),
    }
}

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answered_notes_go_to_the_log_not_away() {
        let (mut req, mut log) = (None, vec![]);
        set_notes(&mut req, &mut log, "shallower");
        set_notes(&mut req, &mut log, "shallower still");
        assert!(log.is_empty(), "an unanswered note is edited, not logged");
        req.as_mut().unwrap().status = RequestStatus::Done;
        set_notes(&mut req, &mut log, "shallower still");
        assert_eq!(req.as_ref().unwrap().status, RequestStatus::Done, "the same text keeps the request");
        set_notes(&mut req, &mut log, "now wider");
        assert_eq!((log.len(), log[0].text.as_str()), (1, "shallower still"));
        assert_eq!(req.as_ref().unwrap().status, RequestStatus::Open);
    }
}
