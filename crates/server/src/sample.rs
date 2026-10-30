//! A made-up SAMPLE game so there's something on screen before a real game is measured.
//! Realistic numbers for a 12-inch square box; none of them were measured.

use crate::spec::{BoxKind, Component, GameInfo, Group, Kind, LayoutChoice, SleeveProfile, Source, Spec, TokenShape, default_printer};

pub const SLUG: &str = "sample-harbour";

pub fn sleeves() -> Vec<SleeveProfile> {
    vec![
        SleeveProfile { id: "penny".into(), name: "Penny sleeve (thin)".into(), per_card_mm: 0.40, source: Source::Estimate },
        SleeveProfile { id: "premium".into(), name: "Premium sleeve (thick)".into(), per_card_mm: 0.55, source: Source::Estimate },
    ]
}

pub fn spec() -> Spec {
    let c = |id: &str, name: &str, group: Option<&str>, kind: Kind| Component {
        id: id.into(),
        name: name.into(),
        group: group.map(Into::into),
        source: Source::Sample,
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
    };
    let mut components = vec![
        c("main", "Game box", None, Kind::Box(BoxKind { inner: [298.0, 298.0, 70.0], lid_depth: 0.0, lid_height: None, lid_lift: 0.0, top_stack: 14.0, layout: LayoutChoice::default(), printed: None })),
        c("main-deck", "Main deck", None, Kind::Cards { size: [66.0, 91.0], count: Some(160), stack: None, sleeve: Some("premium".into()) }),
        c("market-deck", "Market cards", None, Kind::Cards { size: [44.0, 68.0], count: Some(60), stack: None, sleeve: None }),
        c("coins", "Coins", None, Kind::Tokens { shape: TokenShape::Round, size: vec![25.0], thickness: 2.0, count: 50 }),
        c("goods", "Goods tokens", None, Kind::Tokens { shape: TokenShape::Square, size: vec![18.0, 18.0], thickness: 2.2, count: 40 }),
        c("event-deck", "Event cards", None, Kind::Cards { size: [66.0, 91.0], count: Some(90), stack: None, sleeve: Some("penny".into()) }),
        c("harbour-tiles", "Harbour tiles", None, Kind::Tokens { shape: TokenShape::Square, size: vec![45.0, 45.0], thickness: 2.2, count: 30 }),
        c("ships", "Ships", None, Kind::Mini { base: 20.0, height: 16.0, count: 8 }),
        c("dice", "Dice", None, Kind::Bin { size: [60.0, 60.0], depth: 25.0 }),
        c("score-pad", "Score pad", None, Kind::Bin { size: [100.0, 70.0], depth: 12.0 }),
    ];
    let mut groups = vec![];
    for p in 1..=4 {
        let g = format!("p{p}");
        groups.push(Group { id: g.clone(), name: format!("Player {p}") });
        components.push(c(&format!("p{p}-captain"), &format!("P{p} captain"), Some(&g), Kind::Mini { base: 25.0, height: 22.0, count: 1 }));
        components.push(c(&format!("p{p}-cubes"), &format!("P{p} cubes"), Some(&g), Kind::Bin { size: [55.0, 40.0], depth: 18.0 }));
    }
    Spec {
        game: GameInfo {
            name: "SAMPLE: Harbour Guild (made-up game)".into(),
            sample: true,
            notes: Some("Not a real game. Plausible numbers for a 12-inch box so there is something on screen.".into()),
        },
        legacy_boxes: vec![],
        legacy_box: None,
        legacy_layout: None,
        printer: default_printer(),
        sleeves: sleeves(),
        groups,
        components,
        style: "default".into(),
        rules: crate::style::Style::builtin(),
        plates: vec![],
            plate_names: vec![],
            filaments: vec![],
            pieces: vec![],
            labels: Default::default(),
    }
}
