//! The one command surface. The web UI, the `bid` CLI and (later) the MCP tools all send these
//! as JSON to `POST /api/command`. Each validates and returns a structured result or a reason.

use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::spec::{self, BoxKind, Component, GameInfo, Group, Kind, PrintedBox, Request as UserRequest, RequestStatus, SleeveProfile, Source, Spec, Tweak};
use crate::{App, Reason, layout};

#[derive(Deserialize, Debug)]
pub struct CopyIn {
    pub name: Option<String>,
    pub filament: Option<usize>,
}

#[derive(Deserialize, Debug)]
pub struct Request {
    /// The game slug. Defaults to the current game (the last one used).
    #[serde(default)]
    pub game: Option<String>,
    #[serde(flatten)]
    pub cmd: Command,
}

#[derive(Deserialize, Debug)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    /// The games, and which is current.
    ListGames,
    /// Change requests the user left for an agent (tray notes), across every game unless `game`
    /// is given. `status`: open | working | done | declined | pending (open + working, the
    /// default) | all.
    Requests {
        status: Option<String>,
    },
    /// Wait for requests to work on: returns as soon as an open request was made or changed
    /// after `since_ms` (or there's one at all, without it), or after `timeout_s` (default 50,
    /// at most 300) with none. Poll it in a loop, passing back the `now_ms` it returns.
    WaitRequests {
        since_ms: Option<u64>,
        timeout_s: Option<u64>,
    },
    /// Answer a request: `status` working (picked up) | done | declined | open, with a `reply`
    /// saying what was done or why not. `game` and `id` from `requests`.
    UpdateRequest {
        id: String,
        status: RequestStatus,
        reply: Option<String>,
    },
    /// The user has read the answer to a request (done or declined): it leaves the tray for the
    /// tray's log.
    MarkRead {
        id: String,
    },
    /// Log a change the user asked for directly (in a session, not as a tray note) on a tray or
    /// imported piece, once done: `text` what was asked, `reply` what was done.
    LogAsk {
        #[serde(rename = "box")]
        box_: Option<String>,
        tray: String,
        text: String,
        reply: String,
    },
    /// A tray's story, newest first: its requests (open, answered, read), the changes asked for
    /// directly, and the saved changes that name it or what it holds.
    TrayHistory {
        #[serde(rename = "box")]
        box_: Option<String>,
        tray: String,
        limit: Option<usize>,
    },
    /// Edit in SolidCraft: the tray's script for SolidCraft's web app (served at /solidcraft/),
    /// the tray alone in print pose; `commands_no_scoops` if a scoop fillet is refused.
    EditScript {
        #[serde(rename = "box")]
        box_: Option<String>,
        tray: String,
    },
    /// An edited design of a tray from SolidCraft (`design`: the `.sc`, base64). Kept as
    /// `edits/<tray>.sc` and compared with the tray as generated; never taken into the spec.
    SubmitEdit {
        #[serde(rename = "box")]
        box_: Option<String>,
        tray: String,
        design: String,
    },
    /// Print a tray as its last SolidCraft edit, not as generated: a frozen copy of the design.
    /// When the generated tray later changes, the edit's added features go on the new one.
    KeepEdit {
        #[serde(rename = "box")]
        box_: Option<String>,
        tray: String,
    },
    /// Print a tray as generated again, dropping the kept edit (the last edit itself stays).
    DropEdit {
        #[serde(rename = "box")]
        box_: Option<String>,
        tray: String,
    },
    /// The last comparison of a tray's SolidCraft edit with the generated tray, if there is one.
    EditDiff {
        tray: String,
    },
    /// The agents working the request queue: whether one watches from outside (bid, MCP), and
    /// the app's own (its model, thinking level, what it's working on). Never the key itself.
    AgentStatus,
    /// The app's own agent: a Claude API `api_key` (checked before it's kept; `remove_key`
    /// drops it), `model`, `effort` (thinking level) and `enabled` (work the queue).
    SetAgent {
        api_key: Option<String>,
        #[serde(default)]
        remove_key: bool,
        model: Option<String>,
        effort: Option<String>,
        enabled: Option<bool>,
    },
    /// Make `game` current: commands default to it, and browsers not pinned to a game follow.
    UseGame,
    /// A new project. With `inner` it gets its first box; without, add boxes later.
    CreateGame {
        name: String,
        inner: Option<[f64; 3]>,
        #[serde(default)]
        lid_depth: f64,
        #[serde(default)]
        top_stack: f64,
    },
    /// The spec, every box's layout variants, where each component stands, and the latest build.
    GetState,
    SetGame {
        name: Option<String>,
        notes: Option<String>,
    },
    /// A bought box (`inner` measured), or with `printed=true` one designed to print: the packer
    /// sizes it to what goes in it (no larger than `max_inner`, if given), with a slide-on lid.
    AddBox {
        id: String,
        name: String,
        inner: Option<[f64; 3]>,
        #[serde(default)]
        printed: bool,
        max_inner: Option<[f64; 3]>,
        #[serde(default)]
        lid_depth: f64,
        lid_height: Option<f64>,
        #[serde(default)]
        lid_lift: f64,
        #[serde(default)]
        top_stack: f64,
        #[serde(default)]
        source: Source,
    },
    /// Change a box (`box`, default the first).
    SetBox {
        #[serde(rename = "box")]
        box_: Option<String>,
        inner: Option<[f64; 3]>,
        lid_depth: Option<f64>,
        /// The lid's outside height; 0 clears it (full depth).
        lid_height: Option<f64>,
        /// Lid lift allowed, mm; 0 for none.
        lid_lift: Option<f64>,
        top_stack: Option<f64>,
        source: Option<Source>,
        name: Option<String>,
        /// true: designed to print and sized by the packer; false: bought, so give `inner`.
        printed: Option<bool>,
        /// A printed box's largest inside; zeros (or a 0 side) leave it to the bed.
        max_inner: Option<[f64; 3]>,
    },
    /// Remove a box; its components move to the first remaining box.
    RemoveBox {
        id: String,
    },
    AddComponent {
        #[serde(flatten)]
        component: Component,
    },
    /// Merge the other fields into the component; `null` removes an optional field.
    UpdateComponent {
        id: String,
        #[serde(flatten)]
        set: Map<String, Value>,
    },
    RemoveComponent {
        id: String,
    },
    /// Decide how a component is stored: `kind` is cards | tokens | mini | bin | as_is |
    /// undecided. The packer's fields come from its measurements; any given here win.
    StoreAs {
        id: String,
        kind: String,
        #[serde(flatten)]
        set: Map<String, Value>,
    },
    AddGroup {
        id: String,
        name: String,
    },
    /// Removes the group; its components become ungrouped.
    RemoveGroup {
        id: String,
    },
    AddSleeve {
        #[serde(flatten)]
        sleeve: SleeveProfile,
    },
    /// Change the style guide the game is built to: dotted keys such as `walls.normal=2.0` or
    /// `tolerance.print_to_component=0.8`. Every game using that style rebuilds.
    SetStyle {
        #[serde(flatten)]
        values: Map<String, Value>,
    },
    /// Store a photo of a component: `data` is a JPEG or PNG as base64 or a data URL (the `bid`
    /// CLI reads `file=<path>` into it).
    AddPhoto {
        component: String,
        data: String,
        note: Option<String>,
    },
    /// Mark a face on one of the component's photos: corners as fractions of the photo, top-left,
    /// top-right, bottom-right, bottom-left. The viewer textures the placeholder with it.
    SetFace {
        component: String,
        face: String,
        photo: String,
        corners: [[f64; 2]; 4],
    },
    /// Say what a photo shows.
    SetPhotoNote {
        component: String,
        photo: String,
        note: Option<String>,
    },
    /// Delete a photo, and any face cropped from it.
    RemovePhoto {
        component: String,
        photo: String,
    },
    /// Every strategy's layout for a box, compared. Doesn't change the spec.
    GenerateLayouts {
        #[serde(rename = "box")]
        box_: Option<String>,
    },
    ChooseLayout {
        #[serde(rename = "box")]
        box_: Option<String>,
        strategy: String,
    },
    /// Per-tray overrides. Unset fields stay as they are; `clear: true` drops the tray's tweak.
    TweakTray {
        #[serde(rename = "box")]
        box_: Option<String>,
        tray: String,
        max_depth: Option<f64>,
        walls: Option<f64>,
        finger_holes: Option<bool>,
        open_ends: Option<bool>,
        per_pocket: Option<bool>,
        holders: Option<bool>,
        grips: Option<bool>,
        /// "middle", a component id, or "" to remove it.
        half_circle: Option<String>,
        scoops: Option<bool>,
        color: Option<String>,
        /// A stencil label on the tray's top: its text ("" removes it).
        label: Option<String>,
        /// A stencil label on each pocket's floor, naming what goes in it.
        pocket_labels: Option<bool>,
        /// inlay | cut, over the game's `labels.mode`.
        label_mode: Option<spec::LabelMode>,
        /// Pockets along the tray in this order (component ids); [] hands it back.
        order: Option<Vec<String>>,
        /// One pocket, by the component it holds: then `scoops`, `notch` and `notch_keep` (and
        /// `unset` of them) are that pocket's, not the tray's.
        pocket: Option<String>,
        /// A slot this wide (mm) through both long side walls at the pocket's middle; 0 removes it.
        notch: Option<f64>,
        /// How much wall the notch leaves above the pocket's floor, mm (default 5).
        notch_keep: Option<f64>,
        /// With notch_keep 0: how far the notch bites on into the floor past the wall, mm, as a
        /// half-circle cut through it (at most half the notch's width); 0 keeps the floor whole.
        notch_floor: Option<f64>,
        /// The pocket takes the tray's room left over beside it, rather than an empty spare.
        fill_spare: Option<bool>,
        /// Rename the tray; "" goes back to the packer's name.
        name: Option<String>,
        /// Changes wanted, for an agent: a new open request (unchanged text keeps the old one);
        /// "" removes it.
        notes: Option<String>,
        /// Settings to hand back to the packer and style: max_depth, walls, finger_holes,
        /// open_ends, per_pocket, holders, grips, half_circle, scoops, color, name, label,
        /// pocket_labels, label_mode, order.
        #[serde(default)]
        unset: Vec<String>,
        #[serde(default)]
        clear: bool,
    },
    /// Every piece to print, on the printer's plates: where it lies, turned how, and why not.
    Plates,
    /// Put a piece (`<box>/<tray>`, from `plates`) on a plate (1-based; one past the last makes a
    /// new plate), turn it (`rotate` 0/90/180/270 about z) or turn it over (`flip`). Unset fields
    /// stay as they are; `unset` hands fields back to the plate packer; `clear: true` all of them,
    /// and `piece: "*"` with `clear` every piece's.
    SetPlate {
        piece: String,
        plate: Option<usize>,
        rotate: Option<u32>,
        flip: Option<bool>,
        /// Filament slot for the piece (1-based).
        filament: Option<usize>,
        /// Filament slots for named parts of the piece: {"Label": 2}; 0 hands a part back to the
        /// piece's slot.
        #[serde(default)]
        parts: std::collections::BTreeMap<String, usize>,
        #[serde(default)]
        unset: Vec<String>,
        #[serde(default)]
        clear: bool,
    },
    /// Name a plate (1-based); "" clears the name.
    NamePlate {
        plate: usize,
        name: String,
    },
    /// Set filament slot `slot` (1-based; one past the last adds a slot): a Bambu Studio preset
    /// (from `plates`' `presets`) and a colour #rrggbb. Unset fields stay.
    SetFilament {
        slot: usize,
        preset: Option<String>,
        color: Option<String>,
    },
    /// Remove a filament slot. Pieces and parts on it go to slot 1; higher slots move down one.
    RemoveFilament {
        slot: usize,
    },
    /// How the game's stencil labels are made: `mode` inlay (a flush part in its own filament)
    /// or cut (just cut), and `filament`, the inlaid labels' slot (0: back to slot 1). Trays ask
    /// for labels with tweak_tray `label` / `pocket_labels`.
    SetLabels {
        mode: Option<spec::LabelMode>,
        filament: Option<usize>,
    },
    /// Bring in a Bambu Studio project's objects as pieces, as they are (`data`: the 3MF as
    /// base64; the CLI's `file=` reads it): their parts, filament slots, plates and plate names.
    /// `replace: true` drops the pieces, plate names and filaments there now first.
    ImportProject {
        data: String,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        replace: bool,
    },
    /// A piece made in SolidCraft (`design`: its .sc, base64): each body a part, as it lies.
    /// With `piece`, that piece (made in SolidCraft) takes the new design, keeping its place,
    /// copies and filaments; without, a new piece called `name`, not yet placed in the box.
    ImportDesign {
        design: String,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        piece: Option<String>,
    },
    /// The SolidCraft design of a piece made there, as base64, to open it again.
    PieceDesign {
        piece: String,
    },
    /// The copies of an imported piece, one per colour it's printed in: `copies` [{name, filament}].
    /// Places are kept by name.
    SetCopies {
        piece: String,
        copies: Vec<CopyIn>,
    },
    /// Where a copy of an imported piece sits: `copy` (its name; default the first), `at` (min
    /// corner as it sits, mm, in box coordinates or the parent's), `turn` (0/90/180/270 from how it
    /// prints), `flip` (turned over), `inside` (`<piece>` or `<piece>/<copy>`: inside that one;
    /// "" clears), `layer` (the guide layer it shows from), `box`; `unplace: true` takes it out.
    PlacePiece {
        piece: String,
        copy: Option<String>,
        at: Option<[f64; 3]>,
        turn: Option<u32>,
        flip: Option<bool>,
        /// Tipped about x before turning (0/90/180/270): stands a piece printed on its side.
        tilt: Option<u32>,
        inside: Option<String>,
        layer: Option<usize>,
        #[serde(rename = "box")]
        box_: Option<String>,
        #[serde(default)]
        unplace: bool,
    },
    /// Find the compartments of imported pieces in their meshes (`piece`, or every one), for
    /// contents to be put in (update_component pockets=[..]). Replaces what was found before.
    DetectPockets {
        piece: Option<String>,
    },
    /// Remove an imported piece and its meshes.
    RemovePiece {
        piece: String,
    },
    /// Print files for every box's chosen layout into games/<slug>/out/.
    Export,
    /// The game's saved history, newest first: commits (each listing the commands behind it)
    /// and the named versions on them.
    History {
        limit: Option<usize>,
    },
    /// Save what's there now as a named version (and commit anything pending).
    SaveVersion {
        name: String,
        note: Option<String>,
    },
    /// Put the game back as it was at `rev` (a commit or version tag from `history`). What's
    /// there now is committed first, and the restore is a commit of its own, so nothing is lost.
    RestoreVersion {
        rev: String,
    },
}

pub fn ok(game: &str, message: impl Into<String>, data: Value) -> Value {
    json!({ "ok": true, "game": game, "message": message.into(), "data": data })
}

/// What Claude (or the UI) needs to see after a change: the chosen layout, in brief.
pub fn summary(l: &layout::Layout) -> Value {
    json!({
        "strategy": l.strategy,
        "ok": l.ok,
        "trays": l.trays.iter().map(|t| json!({
            "id": t.id, "name": t.name, "request": t.request, "layer": t.layer + 1,
            "size": t.size, "pockets": t.pockets.iter().map(|p| p.label.clone()).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "layers": l.layers.len(),
        "lid_clearance_mm": l.report.lid_clearance.mm,
        "bed_fit": l.report.bed_fit,
        "problems": l.problems,
        "warnings": l.warnings,
    })
}

/// Every box's chosen layout, in brief.
fn summaries(s: &Spec) -> Value {
    Value::Object(s.boxes().iter().map(|gb| (gb.id.clone(), summary(&layout::chosen(s, gb)))).collect())
}

/// Where each component stands: in a tray, in the box as-is, undecided, or not placed (and why).
pub fn placement(s: &Spec) -> Value {
    let boxes = s.boxes();
    let chosen: Vec<_> = boxes.iter().map(|gb| (gb.id.as_str(), layout::chosen(s, gb))).collect();
    let box_name = |id: &str| s.game_box(id).map_or(id.to_string(), |b| b.name.clone());
    let rows: Vec<Value> = s
        .components
        .iter()
        .map(|c| {
            let bx = c.box_in(s);
            let (status, detail, trays): (&str, String, Vec<String>) = match (&c.kind, bx) {
                (_, _) if c.piece.is_some() => {
                    let pid = c.piece.as_deref().unwrap_or_default();
                    match s.pieces.iter().find(|p| p.id == pid) {
                        Some(p) => {
                            let ids: Vec<String> = p.copies.iter().filter(|x| x.at.is_some()).map(|x| crate::pieces::tray_id(p, x)).collect();
                            ("placed", format!("kept in {} (imported as it is)", p.name), ids)
                        }
                        None => ("unplaced", format!("kept in '{pid}', which isn't an imported piece here"), vec![]),
                    }
                }
                (Kind::AsIs, Some(b)) if c.place.is_some() => {
                    let pl = c.place.as_ref().unwrap();
                    ("in_box", format!("lies in the {}{}", box_name(b), pl.layer.map(|l| format!(", guide layer {l}")).unwrap_or_default()), vec![])
                }
                (Kind::Box(b), _) => {
                    let n = s.components.iter().filter(|x| x.box_in(s) == Some(&c.id)).count();
                    let held = format!("holds {n} component{}", if n == 1 { "" } else { "s" });
                    let printed = b.printed.is_some().then(|| chosen.iter().find(|(id, _)| *id == c.id).and_then(|(_, l)| l.printed.as_ref())).flatten();
                    match (b.printed.is_some(), printed) {
                        (true, Some(p)) => ("box", format!("a box to print: {held}; {:.1} × {:.1} × {:.1} mm outside", p.outer[0], p.outer[1], p.outer[2]), vec![]),
                        (true, None) => ("box", format!("a box to print: {held}; sized once it has something to hold"), vec![]),
                        _ => ("box", format!("a box: {held}"), vec![]),
                    }
                }
                (_, None) => ("no_box", "the game has no box yet".into(), vec![]),
                (Kind::Undecided, _) => ("undecided", "how it's stored isn't decided yet (store_as)".into(), vec![]),
                (Kind::AsIs, Some(b)) => ("as_is", format!("goes in the {} as it is, on top of the insert", box_name(b)), vec![]),
                (_, Some(b)) => {
                    let l = &chosen.iter().find(|(id, _)| *id == b).unwrap().1;
                    let held: Vec<_> = l.trays.iter().filter(|t| t.pockets.iter().any(|p| p.component == c.id)).collect();
                    let trays: Vec<String> = held.iter().map(|t| t.id.clone()).collect();
                    if held.is_empty() {
                        let why = l.problems.iter().find(|p| p.subject.as_deref() == Some(&c.id)).map(|p| p.message.clone());
                        ("unplaced", why.unwrap_or_else(|| "the chosen layout has no pocket for it".into()), trays)
                    } else {
                        let pockets: usize = held.iter().map(|t| t.pockets.iter().filter(|p| p.component == c.id).count()).sum();
                        let names: Vec<String> = held.iter().map(|t| format!("{} (layer {})", t.name, t.layer + 1)).collect();
                        let p = if pockets == 1 { "1 pocket".to_string() } else { format!("{pockets} pockets") };
                        ("placed", format!("{p} in {} · {}", names.join(", "), box_name(b)), trays)
                    }
                }
            };
            json!({ "id": c.id, "name": c.name, "kind": c.kind_name(), "box": bx, "status": status, "detail": detail, "trays": trays })
        })
        .collect();
    Value::Array(rows)
}

/// Every request in `games` whose status passes `keep`, newest change first, each with the
/// tray it's about so an agent can act without another call.
pub fn collect_requests(app: &App, games: &[String], keep: impl Fn(RequestStatus) -> bool) -> Vec<Value> {
    let mut out = vec![];
    for g in games {
        let Ok(s) = spec::load(&app.games, g) else { continue };
        for gb in s.boxes() {
            let l = layout::chosen(&s, &gb);
            for t in &gb.layout.tweaks {
                let Some(r) = &t.request else { continue };
                if !keep(r.status) {
                    continue;
                }
                let tray = l.trays.iter().find(|x| x.id == t.tray);
                if t.tray == spec::WHOLE_INSERT {
                    // A note on the insert as a whole: its trays, in brief.
                    out.push(json!({
                        "game": g, "game_name": s.game.name, "box": gb.id, "box_name": gb.name, "tray": t.tray,
                        "tray_name": format!("{} insert (as a whole)", gb.name), "in_layout": true, "strategy": gb.layout.strategy, "request": r,
                        "insert_detail": { "note": "about the whole insert in this box, not one tray", "ok": l.ok, "layers": l.layers.len(),
                            "trays": l.trays.iter().map(|x| json!({ "id": x.id, "name": x.name, "layer": x.layer + 1, "size": x.size })).collect::<Vec<_>>() },
                    }));
                    continue;
                }
                out.push(json!({
                    "game": g, "game_name": s.game.name, "box": gb.id, "box_name": gb.name, "tray": t.tray,
                    "tray_name": tray.map(|x| x.name.clone()), "in_layout": tray.is_some(), "strategy": gb.layout.strategy,
                    "request": r,
                    "tray_detail": tray.map(|x| json!({
                        "layer": x.layer + 1, "size": x.size, "wall": x.wall, "floor": x.floor,
                        "pockets": x.pockets.iter().map(|p| json!({ "label": p.label, "component": p.component, "size": p.size, "depth": p.depth, "items": p.items })).collect::<Vec<_>>(),
                        "tweak": t,
                    })),
                }));
            }
        }
        for p in &s.pieces {
            let Some(r) = &p.request else { continue };
            if !keep(r.status) {
                continue;
            }
            out.push(json!({
                "game": g, "game_name": s.game.name, "box": p.box_, "tray": p.copies.first().map(|c| crate::pieces::tray_id(p, c)), "tray_name": p.name,
                "in_layout": true, "imported": true, "request": r,
                "tray_detail": { "imported": true, "piece": p.id, "size": p.size, "parts": p.parts.iter().map(|x| &x.name).collect::<Vec<_>>(), "source": p.source,
                    "note": "an imported piece: its geometry isn't generated, so a change to its shape needs a new import" },
            }));
        }
    }
    out.sort_by_key(|v| std::cmp::Reverse(v["request"]["updated_ms"].as_u64().unwrap_or(0)));
    out
}

fn status_filter(f: Option<&str>) -> Result<Box<dyn Fn(RequestStatus) -> bool>, Reason> {
    Ok(match f.unwrap_or("pending") {
        "pending" => Box::new(|s: RequestStatus| s.pending()),
        "all" => Box::new(|_| true),
        other => {
            let want: RequestStatus = serde_json::from_value(json!(other))
                .map_err(|_| Reason::new("bad_status", "status is open, working, done, declined, pending or all"))?;
            Box::new(move |s| s == want)
        }
    })
}

pub async fn run(app: &std::sync::Arc<App>, req: Request) -> Result<Value, Reason> {
    let games = spec::list_games(&app.games);
    let scope = || match &req.game {
        Some(g) => vec![g.clone()],
        None => games.clone(),
    };
    if let Command::Requests { status } = &req.cmd {
        let rows = collect_requests(app, &scope(), status_filter(status.as_deref())?);
        return Ok(json!({ "ok": true, "message": format!("{} requests", rows.len()), "data": { "requests": rows, "now_ms": crate::now_ms() } }));
    }
    if let Command::WaitRequests { since_ms, timeout_s } = &req.cmd {
        // Subscribe before looking, so a change between the look and the wait isn't missed.
        let mut rx = app.tx.subscribe();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_s.unwrap_or(50).clamp(1, 300));
        let since = since_ms.unwrap_or(0);
        loop {
            let now = crate::now_ms();
            let rows: Vec<Value> = collect_requests(app, &scope(), |s| s == RequestStatus::Open)
                .into_iter()
                .filter(|v| v["request"]["updated_ms"].as_u64().unwrap_or(0) > since)
                .collect();
            if !rows.is_empty() {
                return Ok(json!({ "ok": true, "message": format!("{} open requests", rows.len()), "data": { "requests": rows, "now_ms": now } }));
            }
            tokio::select! {
                m = rx.recv() => if m.is_err() && rx.is_closed() { break },
                _ = tokio::time::sleep_until(deadline) => break,
            }
        }
        return Ok(json!({ "ok": true, "message": "no new requests", "data": { "requests": [], "now_ms": crate::now_ms() } }));
    }

    if let Command::AgentStatus = req.cmd {
        return Ok(json!({ "ok": true, "message": "agent status", "data": app.agent.status() }));
    }
    if let Command::SetAgent { api_key, remove_key, model, effort, enabled } = req.cmd {
        let msg = app.agent.set(api_key, remove_key, model, effort, enabled).await?;
        let status = app.agent.status();
        app.emit(json!({ "type": "agent", "status": status }));
        return Ok(json!({ "ok": true, "message": msg, "data": status }));
    }
    if let Command::ListGames = req.cmd {
        let current = app.current().await;
        return Ok(json!({ "ok": true, "message": format!("{} games", games.len()), "data": { "games": games, "current": current } }));
    }
    if let Command::CreateGame { name, inner, lid_depth, top_stack } = &req.cmd {
        // The slug defaults to one made from the name.
        let slug = req.game.clone().unwrap_or_else(|| slugify(name));
        if !spec::is_slug(&slug) {
            return Err(Reason::new("bad_id", "a game slug is a-z, 0-9 and -").subject(&slug));
        }
        if games.contains(&slug) {
            return Err(Reason::new("exists", format!("game '{slug}' already exists")).subject(&slug));
        }
        let s = Spec {
            game: GameInfo { name: name.clone(), sample: false, notes: None },
            legacy_boxes: vec![],
            legacy_box: None,
            legacy_layout: None,
            printer: spec::default_printer(),
            sleeves: crate::sample::sleeves(),
            groups: vec![],
            components: inner
                .map(|inner| {
                    let b = BoxKind { inner, lid_depth: *lid_depth, lid_height: None, lid_lift: 0.0, top_stack: *top_stack, layout: Default::default(), printed: None };
                    Component::new("main", "Game box", Source::Measured, Kind::Box(b))
                })
                .into_iter()
                .collect(),
            style: "default".into(),
            rules: crate::style::load("default")?,
            plates: vec![],
            plate_names: vec![],
            filaments: vec![],
            labels: Default::default(),
            pieces: vec![],
        };
        s.validate()?;
        spec::save(&app.games, &slug, &s)?;
        app.set_current(&slug).await;
        app.changed(&slug).await?;
        return Ok(ok(&slug, format!("created {slug}; it is now the current game"), json!({})));
    }

    let game = match req.game.clone() {
        Some(g) => g,
        None => app.current().await.ok_or_else(|| {
            Reason::new("game_required", format!("no current game; pass game=<slug> (have: {})", games.join(", ")))
        })?,
    };
    if !games.contains(&game) {
        return Err(Reason::new("no_game", format!("no game '{game}' (have: {})", games.join(", "))).subject(&game));
    }

    match req.cmd {
        Command::UseGame => {
            app.set_current(&game).await;
            app.changed(&game).await?;
            Ok(ok(&game, format!("{game} is now the current game"), json!({})))
        }
        Command::GetState => Ok(ok(&game, "state", app.state_json(&game).await?)),
        Command::GenerateLayouts { box_ } => {
            let s = spec::load(&app.games, &game)?;
            let gb = s.pick_box(box_.as_deref())?;
            let all = layout::generate_all(&s, &gb);
            let rows: Vec<Value> = all
                .iter()
                .map(|l| {
                    let mut v = summary(l);
                    v["title"] = json!(l.title);
                    v["chosen"] = json!(l.strategy == gb.layout.strategy);
                    v
                })
                .collect();
            Ok(ok(&game, format!("{}: {} layout variants; chosen: {}", gb.id, rows.len(), gb.layout.strategy), json!({ "box": gb.id, "layouts": rows })))
        }
        Command::SetStyle { values } => {
            let name = spec::load(&app.games, &game)?.style;
            if values.is_empty() {
                let st = crate::style::load(&name)?;
                return Ok(ok(&game, format!("style '{name}'"), serde_json::to_value(st).unwrap()));
            }
            let st = crate::style::set(&name, &values)?;
            for g in spec::list_games(&app.games) {
                if spec::load(&app.games, &g).map(|s| s.style == name).unwrap_or(false) {
                    let _ = app.changed(&g).await;
                }
            }
            let s = spec::load(&app.games, &game)?;
            let keys: Vec<_> = values.keys().cloned().collect();
            let mut data = json!({ "layouts": summaries(&s) });
            data["style"] = serde_json::to_value(&st).unwrap();
            Ok(ok(&game, format!("style '{name}': set {}", keys.join(", ")), data))
        }
        Command::AddPhoto { component, data, note } => {
            let s = spec::load(&app.games, &game)?;
            if s.component(&component).is_none() {
                return Err(no_component(&s, &component));
            }
            let file = crate::media::save_photo(&app.games, &game, &component, &data)?;
            let f = file.clone();
            let msg = app
                .mutate(&game, move |s| {
                    let c = s.components.iter_mut().find(|c| c.id == component).ok_or_else(|| Reason::new("no_component", "gone"))?;
                    c.photos.push(spec::Photo { file: f.clone(), note });
                    Ok(format!("added photo {f} to '{}'", c.id))
                })
                .await?;
            Ok(ok(&game, msg, json!({ "photo": file, "url": format!("/api/games/{game}/media/photos/{file}") })))
        }
        Command::SetFace { component, face, photo, corners } => {
            let (c2, f2) = (component.clone(), face.clone());
            let msg = app
                .mutate(&game, move |s| {
                    let c = s.components.iter_mut().find(|c| c.id == c2).ok_or_else(|| Reason::new("no_component", format!("no component '{c2}'")))?;
                    let allowed = crate::media::faces_for(c);
                    if !allowed.contains(&f2.as_str()) {
                        return Err(Reason::new("bad_face", format!("a {} shows faces: {}", c.kind_name(), allowed.join(", "))).subject(&c2));
                    }
                    c.faces.insert(f2.clone(), spec::Face { photo, corners });
                    Ok(format!("set the {f2} face of '{c2}'"))
                })
                .await?;
            let s = spec::load(&app.games, &game)?;
            let c = s.component(&component).unwrap();
            crate::media::render_face(&app.games, &game, c, &face, &c.faces[&face])?;
            app.changed(&game).await?;
            Ok(ok(&game, msg, json!({ "texture": format!("/api/games/{game}/media/faces/{component}-{face}.jpg") })))
        }
        Command::SetPhotoNote { component, photo, note } => {
            let msg = app
                .mutate(&game, move |s| {
                    let c = s.components.iter_mut().find(|c| c.id == component).ok_or_else(|| Reason::new("no_component", format!("no component '{component}'")))?;
                    let p = c.photos.iter_mut().find(|p| p.file == photo).ok_or_else(|| Reason::new("no_photo", format!("'{component}' has no photo {photo}")).subject(&component))?;
                    p.note = note.filter(|n| !n.trim().is_empty());
                    Ok(format!("noted photo {photo}"))
                })
                .await?;
            Ok(ok(&game, msg, json!({})))
        }
        Command::RemovePhoto { component, photo } => {
            let (c2, p2) = (component.clone(), photo.clone());
            let msg = app
                .mutate(&game, move |s| {
                    let c = s.components.iter_mut().find(|c| c.id == c2).ok_or_else(|| Reason::new("no_component", format!("no component '{c2}'")))?;
                    let before = c.photos.len();
                    c.photos.retain(|p| p.file != p2);
                    if c.photos.len() == before {
                        return Err(Reason::new("no_photo", format!("'{c2}' has no photo {p2}")).subject(&c2));
                    }
                    let dropped: Vec<String> = c.faces.iter().filter(|(_, f)| f.photo == p2).map(|(k, _)| k.clone()).collect();
                    c.faces.retain(|_, f| f.photo != p2);
                    Ok(format!("removed photo {p2}{}", if dropped.is_empty() { String::new() } else { format!(" and its {} face", dropped.join(", ")) }))
                })
                .await?;
            let _ = std::fs::remove_file(crate::media::photo_path(&app.games, &game, &photo));
            Ok(ok(&game, msg, json!({})))
        }
        Command::UpdateRequest { id, status, reply } => {
            let msg = app
                .mutate(&game, move |s| {
                    let r = s
                        .components
                        .iter_mut()
                        .filter_map(|c| match &mut c.kind {
                            Kind::Box(b) => Some(&mut b.layout.tweaks),
                            _ => None,
                        })
                        .flatten()
                        .filter_map(|t| t.request.as_mut())
                        .chain(s.pieces.iter_mut().filter_map(|p| p.request.as_mut()))
                        .find(|r| r.id == id)
                        .ok_or_else(|| Reason::new("no_request", format!("no request '{id}' in this game (requests lists them)")).subject(&id))?;
                    r.status = status;
                    r.updated_ms = crate::now_ms();
                    if let Some(v) = reply {
                        r.reply = Some(v.trim().to_string()).filter(|v| !v.is_empty());
                    }
                    Ok(format!("request {id} is {}", serde_json::to_value(status).unwrap().as_str().unwrap_or("")))
                })
                .await?;
            Ok(ok(&game, msg, json!({})))
        }
        Command::MarkRead { id } => {
            let msg = app
                .mutate(&game, move |s| {
                    let mut slots: Vec<(&mut Option<UserRequest>, &mut Vec<UserRequest>)> = s
                        .components
                        .iter_mut()
                        .filter_map(|c| match &mut c.kind {
                            Kind::Box(b) => Some(&mut b.layout.tweaks),
                            _ => None,
                        })
                        .flatten()
                        .map(|t| (&mut t.request, &mut t.log))
                        .chain(s.pieces.iter_mut().map(|p| (&mut p.request, &mut p.log)))
                        .collect();
                    let (req, log) = slots
                        .iter_mut()
                        .find(|(r, _)| r.as_ref().is_some_and(|r| r.id == id))
                        .ok_or_else(|| Reason::new("no_request", format!("no request '{id}' on a tray in this game")).subject(&id))?;
                    if req.as_ref().is_some_and(|r| r.status.pending()) {
                        return Err(Reason::new("not_answered", "Claude hasn't answered it yet: it's still open or being worked on").subject(&id));
                    }
                    let mut r = req.take().unwrap();
                    r.read_ms = Some(crate::now_ms());
                    log.push(r);
                    Ok(format!("request {id} read: moved to the tray's history"))
                })
                .await?;
            Ok(ok(&game, msg, json!({})))
        }
        Command::LogAsk { box_, tray, text, reply } => {
            let msg = app
                .mutate(&game, move |s| {
                    let (text, reply) = (text.trim().to_string(), reply.trim().to_string());
                    if text.is_empty() || reply.is_empty() {
                        return Err(Reason::new("bad_text", "say what was asked (text) and what was done (reply)"));
                    }
                    let mut r = UserRequest::new(text);
                    (r.status, r.reply, r.direct) = (RequestStatus::Done, Some(reply), true);
                    r.id = format!("a{}", r.created_ms);
                    if let Some(p) = s.pieces.iter_mut().find(|p| p.copies.iter().any(|c| crate::pieces::tray_id(p, c) == tray)) {
                        p.log.push(r);
                        return Ok(format!("logged an ask on '{tray}'"));
                    }
                    let gb = s.pick_box(box_.as_deref())?;
                    if tray != spec::WHOLE_INSERT && !layout::chosen(s, &gb).trays.iter().any(|t| t.id == tray) {
                        return Err(Reason::new("no_tray", format!("no tray '{tray}' in {}", gb.id)).subject(&tray));
                    }
                    let tweaks = &mut s.pick_box_mut(box_.as_deref())?.box_kind_mut().layout.tweaks;
                    match tweaks.iter_mut().find(|t| t.tray == tray) {
                        Some(t) => t.log.push(r),
                        None => tweaks.push(Tweak { tray: tray.clone(), log: vec![r], ..Default::default() }),
                    }
                    Ok(format!("logged an ask on '{tray}'"))
                })
                .await?;
            Ok(ok(&game, msg, json!({})))
        }
        Command::EditScript { box_, tray } => {
            let s = spec::load(&app.games, &game)?;
            let gb = s.pick_box(box_.as_deref())?;
            let l = layout::chosen(&s, &gb);
            let t = l.trays.iter().find(|t| t.id == tray).ok_or_else(|| Reason::new("no_tray", format!("no tray '{tray}' in {}", gb.id)).subject(&tray))?;
            let (with, without) = crate::edit::script(&l, t);
            // A tray kept as an edit opens as that design, to go on from it.
            let design = match t.kept_edit.as_ref().and_then(|_| crate::edit::kept_design(&app.games, &game, &tray)) {
                Some(p) => Some(crate::edit::design_steps(&app.cli, &p).await?),
                None => None,
            };
            let msg = format!("{tray}: {} commands for SolidCraft{}", with.len(), if design.is_some() { ", and the kept edit's" } else { "" });
            Ok(ok(&game, msg, json!({ "tray": tray, "box": gb.id, "solidcraft": app.solidcraft_tag, "commands": with, "commands_no_scoops": without, "design_commands": design })))
        }
        Command::KeepEdit { box_, tray } => {
            let kept = crate::edit::keep(&app.games, &game, &tray)?;
            let (a, c) = (kept.added, kept.changed);
            let msg = app
                .mutate(&game, move |s| {
                    let tweaks = &mut s.pick_box_mut(box_.as_deref())?.box_kind_mut().layout.tweaks;
                    match tweaks.iter_mut().find(|t| t.tray == tray) {
                        Some(t) => t.edit = Some(kept),
                        None => tweaks.push(Tweak { tray: tray.clone(), edit: Some(kept), ..Default::default() }),
                    }
                    let carry = if c > 0 { format!("; if the generated tray changes, the {a} feature(s) you added go on the new one, but not the {c} change(s) to its own features") } else { format!("; if the generated tray changes, the {a} feature(s) you added go on the new one") };
                    Ok(format!("'{tray}' prints as your SolidCraft edit{carry}"))
                })
                .await?;
            Ok(ok(&game, msg, json!({})))
        }
        Command::DropEdit { box_, tray } => {
            let (games_dir, g2, t2) = (app.games.clone(), game.clone(), tray.clone());
            let msg = app
                .mutate(&game, move |s| {
                    let tweaks = &mut s.pick_box_mut(box_.as_deref())?.box_kind_mut().layout.tweaks;
                    let t = tweaks.iter_mut().find(|t| t.tray == tray && t.edit.is_some()).ok_or_else(|| Reason::new("no_edit", format!("'{tray}' isn't kept as an edit")).subject(&tray))?;
                    t.edit = None;
                    Ok(format!("'{tray}' prints as generated again"))
                })
                .await?;
            crate::edit::drop_kept(&games_dir, &g2, &t2);
            Ok(ok(&game, msg, json!({})))
        }
        Command::SubmitEdit { box_, tray, design } => {
            use base64::Engine;
            let raw = design.split_once("base64,").map_or(design.as_str(), |(_, b)| b);
            let bytes = base64::engine::general_purpose::STANDARD.decode(raw.trim()).map_err(|e| Reason::new("bad_data", format!("not base64: {e}")))?;
            let s = spec::load(&app.games, &game)?;
            let gb = s.pick_box(box_.as_deref())?;
            let l = layout::chosen(&s, &gb);
            let t = l.trays.iter().find(|t| t.id == tray).ok_or_else(|| Reason::new("no_tray", format!("no tray '{tray}' in {}", gb.id)).subject(&tray))?;
            crate::edit::observe(&app.cli, &app.games, &game, &l, t, &app.solidcraft_tag, &bytes).await?;
            let d = crate::edit::last(&app.games, &game, &tray).unwrap_or_default();
            app.history.note(&game, &format!("submit_edit: SolidCraft edit of '{tray}' kept")).await;
            app.emit(json!({ "type": "edit", "game": game, "tray": tray }));
            let n = d["summary"].as_array().map_or(0, Vec::len);
            Ok(ok(&game, format!("{tray}: edit kept, {n} differences from the generated tray"), d))
        }
        Command::EditDiff { tray } => match crate::edit::last(&app.games, &game, &tray) {
            Some(d) => Ok(ok(&game, format!("{tray}: last SolidCraft edit"), d)),
            None => Ok(ok(&game, format!("{tray}: no SolidCraft edit"), Value::Null)),
        },
        Command::TrayHistory { box_, tray, limit } => {
            let s = spec::load(&app.games, &game)?;
            let mut rows: Vec<Value> = vec![];
            let entry = |r: &UserRequest| {
                let kind = if r.direct { "ask" } else { "note" };
                json!({ "kind": kind, "time_ms": r.read_ms.unwrap_or(r.updated_ms), "asked_ms": r.created_ms, "request": r })
            };
            // What names this tray in a saved change: it, its frame, or what it holds.
            let mut names = vec![tray.clone()];
            if let Some(p) = s.pieces.iter().find(|p| p.copies.iter().any(|c| crate::pieces::tray_id(p, c) == tray)) {
                rows.extend(p.log.iter().chain(&p.request).map(entry));
                names.push(p.id.clone());
            } else if tray == spec::WHOLE_INSERT {
                let gb = s.pick_box(box_.as_deref())?;
                if let Some(w) = gb.layout.tweaks.iter().find(|w| w.tray == tray) {
                    rows.extend(w.log.iter().chain(&w.request).map(entry));
                }
            } else {
                let gb = s.pick_box(box_.as_deref())?;
                let l = layout::chosen(&s, &gb);
                let t = l.trays.iter().find(|t| t.id == tray).ok_or_else(|| Reason::new("no_tray", format!("no tray '{tray}' in {}", gb.id)).subject(&tray))?;
                names.extend(t.holder_of.clone());
                names.extend(t.pockets.iter().filter(|p| !p.component.is_empty()).map(|p| p.component.clone()));
                if let Some(w) = gb.layout.tweaks.iter().find(|w| w.tray == tray) {
                    rows.extend(w.log.iter().chain(&w.request).map(entry));
                }
            }
            let quoted: Vec<String> = names.iter().map(|n| format!("'{n}'")).collect();
            for c in app.history.log(&game, limit.unwrap_or(300).min(1000)).await? {
                let lines: Vec<String> = match c["changes"].as_array().filter(|a| !a.is_empty()) {
                    Some(a) => a.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
                    None => vec![c["subject"].as_str().unwrap_or("").to_string()],
                };
                for line in lines {
                    // Requests show as themselves above, not as the edits that carried them.
                    let theirs = ["update_request:", "mark_read:", "log_ask:"].iter().any(|p| line.starts_with(p));
                    if !theirs && (tray == spec::WHOLE_INSERT || quoted.iter().any(|q| line.contains(q.as_str()))) {
                        rows.push(json!({ "kind": "change", "time_ms": c["time_ms"], "text": line, "commit": c["short"] }));
                    }
                }
            }
            rows.sort_by_key(|v| std::cmp::Reverse(v["time_ms"].as_u64().unwrap_or(0)));
            Ok(ok(&game, format!("{} entries for '{tray}'", rows.len()), json!({ "history": rows })))
        }
        Command::History { limit } => {
            let rows = app.history.log(&game, limit.unwrap_or(100).min(1000)).await?;
            Ok(ok(&game, format!("{} saved changes", rows.len()), json!({ "history": rows, "status": app.history.status(&game).await })))
        }
        Command::SaveVersion { name, note } => {
            let name = name.trim().to_string();
            if name.is_empty() {
                return Err(Reason::new("bad_name", "give the version a name"));
            }
            let v = app.history.save_version(&game, &name, note.as_deref()).await?;
            app.emit(json!({ "type": "history", "game": game }));
            Ok(ok(&game, format!("saved version '{name}' as {}", v["tag"].as_str().unwrap_or("")), v))
        }
        Command::RestoreVersion { rev } => {
            let msg = {
                let _w = app.write.lock().await;
                let msg = app.history.restore_files(&game, &rev).await?;
                if let Err(e) = spec::load(&app.games, &game) {
                    // Shouldn't happen (it was valid when committed), but say so rather than leave a broken game.
                    return Err(Reason::new("restore_invalid", format!("{msg}, but it doesn't load now: {}", e.message)));
                }
                app.history.commit(&game, Some(&msg)).await?;
                msg
            };
            app.history.push().await;
            app.changed(&game).await?;
            app.emit(json!({ "type": "history", "game": game }));
            Ok(ok(&game, msg, json!({})))
        }
        Command::ImportProject { data, name, replace } => {
            use base64::Engine;
            let raw = data.split_once("base64,").map_or(data.as_str(), |(_, b)| b);
            let bytes = base64::engine::general_purpose::STANDARD.decode(raw.trim()).map_err(|e| Reason::new("bad_data", format!("not base64: {e}")))?;
            let file = name.unwrap_or_else(|| "project.3mf".into());
            let s = spec::load(&app.games, &game)?;
            let taken: Vec<String> = if replace { vec![] } else { s.pieces.iter().map(|p| p.id.clone()).collect() };
            let im = crate::pieces::import(&app.games, &game, &bytes, &file, &taken)?;
            let n = im.pieces.len();
            let plates = im.plate_names.len();
            let (games_dir, g2) = (app.games.clone(), game.clone());
            let msg = app
                .mutate(&game, move |s| {
                    if replace {
                        for p in &s.pieces {
                            if !im.pieces.iter().any(|q| q.id == p.id) {
                                let _ = std::fs::remove_dir_all(crate::pieces::dir(&games_dir, &g2, &p.id));
                            }
                        }
                        s.pieces.clear();
                        s.plates.retain(|p| !p.piece.starts_with("piece/"));
                    }
                    if replace || s.filaments.is_empty() {
                        s.filaments = im.filaments;
                        s.plate_names = im.plate_names;
                    }
                    s.plates.extend(im.pins);
                    s.pieces.extend(im.pieces);
                    Ok(format!("imported {n} pieces from {file}, on {plates} plates"))
                })
                .await?;
            let s = spec::load(&app.games, &game)?;
            Ok(ok(&game, msg, json!({ "pieces": s.pieces.iter().map(|p| json!({ "id": p.id, "name": p.name, "size": p.size, "parts": p.parts.len() })).collect::<Vec<_>>() })))
        }
        Command::ImportDesign { design, name, piece } => {
            use base64::Engine;
            let raw = design.split_once("base64,").map_or(design.as_str(), |(_, b)| b);
            let bytes = base64::engine::general_purpose::STANDARD.decode(raw.trim()).map_err(|e| Reason::new("bad_data", format!("not base64: {e}")))?;
            let s = spec::load(&app.games, &game)?;
            let id = match &piece {
                Some(id) => {
                    let p = s.pieces.iter().find(|p| &p.id == id).ok_or_else(|| Reason::new("no_piece", format!("no piece '{id}'")).subject(id))?;
                    if p.design.is_none() {
                        return Err(Reason::new("not_designed", format!("'{id}' came from a Bambu Studio project, not SolidCraft: its parts can't be replaced by a design")).subject(id));
                    }
                    id.clone()
                }
                None => {
                    let base = slugify(name.as_deref().unwrap_or("piece"));
                    let base = if base.is_empty() { "piece".to_string() } else { base };
                    let (mut id, mut k) = (base.clone(), 2);
                    while s.pieces.iter().any(|p| p.id == id) || crate::pieces::dir(&app.games, &game, &id).exists() {
                        id = format!("{base}-{k}");
                        k += 1;
                    }
                    id
                }
            };
            let (parts, size) = crate::pieces::from_design(&app.cli, &app.games, &game, &id, &bytes).await?;
            let (games_dir, g2, id2) = (app.games.clone(), game.clone(), id.clone());
            let label = name.clone().unwrap_or_else(|| id.clone());
            let msg = app
                .mutate(&game, move |s| {
                    let n = parts.len();
                    let made = match s.pieces.iter_mut().find(|p| p.id == id2) {
                        Some(p) => {
                            (p.parts, p.size, p.design) = (parts, size, Some(crate::pieces::DESIGN.into()));
                            false
                        }
                        None => {
                            s.pieces.push(spec::Piece {
                                id: id2.clone(),
                                name: label,
                                source: Some("SolidCraft design".into()),
                                size,
                                filament: 1,
                                parts,
                                box_: None,
                                auto_name: None,
                                request: None,
                                log: vec![],
                                pockets: vec![],
                                copies: vec![spec::PieceCopy::default()],
                                design: Some(crate::pieces::DESIGN.into()),
                            });
                            true
                        }
                    };
                    let p = s.pieces.iter_mut().find(|p| p.id == id2).unwrap();
                    p.pockets = crate::pieces::detect_pockets(&games_dir, &g2, p).unwrap_or_default();
                    let [x, y, z] = size;
                    Ok(format!("{} '{id2}' from a SolidCraft design: {n} part{}, {x:.1} × {y:.1} × {z:.1} mm", if made { "made" } else { "updated" }, if n == 1 { "" } else { "s" }))
                })
                .await?;
            Ok(ok(&game, msg, json!({ "piece": id, "size": size })))
        }
        Command::PieceDesign { piece } => {
            use base64::Engine;
            let s = spec::load(&app.games, &game)?;
            let p = s.pieces.iter().find(|p| p.id == piece).ok_or_else(|| Reason::new("no_piece", format!("no piece '{piece}'")).subject(&piece))?;
            let file = p.design.as_ref().ok_or_else(|| Reason::new("not_designed", format!("'{piece}' wasn't made in SolidCraft")).subject(&piece))?;
            let path = crate::pieces::dir(&app.games, &game, &piece).join(file);
            let bytes = std::fs::read(&path).map_err(|e| Reason::new("io", e.to_string()))?;
            let data = base64::engine::general_purpose::STANDARD.encode(bytes);
            let steps = crate::edit::design_steps(&app.cli, &path).await?;
            Ok(ok(&game, format!("{piece}'s SolidCraft design"), json!({ "piece": piece, "name": p.name, "design": data, "commands": steps })))
        }
        Command::DetectPockets { piece } => {
            let s = spec::load(&app.games, &game)?;
            let mut found = vec![];
            for p in s.pieces.iter().filter(|p| piece.as_ref().is_none_or(|x| *x == p.id)) {
                let ps = crate::pieces::detect_pockets(&app.games, &game, p).map_err(|e| Reason::new("io", e).subject(&p.id))?;
                found.push((p.id.clone(), ps));
            }
            if found.is_empty() {
                return Err(Reason::new("no_piece", "no imported piece by that id"));
            }
            let summary: Vec<Value> = found.iter().map(|(id, ps)| json!({ "piece": id, "pockets": ps })).collect();
            let n: usize = found.iter().map(|(_, ps)| ps.len()).sum();
            let msg = app
                .mutate(&game, move |s| {
                    for (id, ps) in found {
                        if let Some(p) = s.pieces.iter_mut().find(|p| p.id == id) {
                            p.pockets = ps;
                        }
                    }
                    Ok(format!("found {n} pockets"))
                })
                .await?;
            Ok(ok(&game, msg, json!({ "pieces": summary })))
        }
        Command::Plates => {
            let s = spec::load(&app.games, &game)?;
            let pl = app.plates_json(&game, &s);
            Ok(ok(&game, format!("{} pieces on {} plates", pl["pieces"].as_array().map_or(0, |a| a.len()), pl["count"]), pl))
        }
        Command::Export => {
            let v = app.export(&game).await?;
            Ok(ok(&game, format!("wrote {} files to games/{game}/out/", v["files"].as_array().map_or(0, |a| a.len())), v))
        }
        cmd @ (Command::SetPlate { .. } | Command::NamePlate { .. } | Command::SetFilament { .. } | Command::RemoveFilament { .. } | Command::SetLabels { .. }) => {
            let msg = app.mutate(&game, |s| apply(s, cmd)).await?;
            let s = spec::load(&app.games, &game)?;
            Ok(ok(&game, msg, app.plates_json(&game, &s)))
        }
        cmd => {
            let msg = app.mutate(&game, |s| apply(s, cmd)).await?;
            let s = spec::load(&app.games, &game)?;
            Ok(ok(&game, msg, json!({ "layouts": summaries(&s) })))
        }
    }
}

fn apply(s: &mut Spec, cmd: Command) -> Result<String, Reason> {
    match cmd {
        Command::SetGame { name, notes } => {
            if let Some(v) = name {
                s.game.name = v;
            }
            if let Some(v) = notes {
                s.game.notes = Some(v).filter(|n| !n.trim().is_empty());
            }
            Ok(format!("project is '{}'", s.game.name))
        }
        Command::AddBox { id, name, inner, printed, max_inner, lid_depth, lid_height, lid_lift, top_stack, source } => {
            if s.game_box(&id).is_some() {
                return Err(Reason::new("exists", format!("box '{id}' exists; use set_box")).subject(&id));
            }
            if s.component(&id).is_some() {
                return Err(Reason::new("exists", format!("component '{id}' exists; mark it as a box with store_as kind=box")).subject(&id));
            }
            let (b, msg) = if printed {
                let pb = PrintedBox { max_inner: max_inner.filter(|m| m.iter().any(|v| *v > 0.0)) };
                (BoxKind { inner: [0.0; 3], lid_depth: 0.0, lid_height: None, lid_lift: 0.0, top_stack, layout: Default::default(), printed: Some(pb) }, format!("added box '{id}' to print, sized to what goes in it"))
            } else {
                let inner = inner.ok_or_else(|| Reason::new("needs_inner", "a bought box needs its inside measured: inner [length, width, height] mm (or printed=true)").subject(&id))?;
                (BoxKind { inner, lid_depth, lid_height, lid_lift, top_stack, layout: Default::default(), printed: None }, format!("added box '{id}', {inner:?} mm inside"))
            };
            let at = s.box_slot();
            s.components.insert(at, Component::new(&id, &name, if printed { Source::Default } else { source }, Kind::Box(b)));
            Ok(msg)
        }
        Command::SetBox { box_, inner, lid_depth, lid_height, lid_lift, top_stack, source, name, printed, max_inner } => {
            let c = s.pick_box_mut(box_.as_deref())?;
            if let Some(v) = source {
                c.source = v;
            }
            if let Some(v) = name {
                c.name = v;
            }
            let id = c.id.clone();
            let b = c.box_kind_mut();
            if let Some(v) = inner {
                b.inner = v;
            }
            if let Some(v) = lid_depth {
                b.lid_depth = v;
            }
            if let Some(v) = lid_height {
                b.lid_height = Some(v).filter(|v| *v > 0.0);
            }
            if let Some(v) = lid_lift {
                b.lid_lift = v;
            }
            if let Some(v) = top_stack {
                b.top_stack = v;
            }
            match printed {
                Some(true) if b.printed.is_none() => b.printed = Some(PrintedBox::default()),
                Some(false) if b.printed.is_some() => {
                    if b.inner == [0.0; 3] {
                        return Err(Reason::new("needs_inner", "a bought box needs its inside measured: give inner with printed=false").subject(&id));
                    }
                    b.printed = None;
                }
                _ => {}
            }
            if let Some(m) = max_inner {
                let pb = b.printed.as_mut().ok_or_else(|| Reason::new("not_printed", "max_inner is for a printed box (printed=true)").subject(&id))?;
                pb.max_inner = Some(m).filter(|m| m.iter().any(|v| *v > 0.0));
            }
            if b.printed.is_some() {
                // The packer sizes a printed box; a measured inside would only mislead.
                b.inner = [0.0; 3];
                return Ok(format!("box '{id}' is printed, sized to its insert{}, top stack {}", b.printed.as_ref().and_then(|p| p.max_inner).map(|m| format!(" (at most {m:?} mm inside)")).unwrap_or_default(), b.top_stack));
            }
            Ok(format!("box '{id}' is {:?} mm, lid depth {}, top stack {}, lid lift allowed {}", b.inner, b.lid_depth, b.top_stack, b.lid_lift))
        }
        Command::RemoveBox { id } => {
            if !s.component(&id).is_some_and(|c| c.is_box()) {
                return Err(Reason::new("no_box", format!("no box '{id}'")).subject(&id));
            }
            apply(s, Command::RemoveComponent { id })
        }
        Command::AddComponent { component } => {
            if s.component(&component.id).is_some() {
                return Err(Reason::new("exists", format!("component '{}' exists; use update_component", component.id)).subject(&component.id));
            }
            let msg = format!("added {} '{}'", kind_name(&component), component.id);
            if component.is_box() {
                let at = s.box_slot();
                s.components.insert(at, component);
            } else {
                s.components.push(component);
            }
            Ok(msg)
        }
        Command::UpdateComponent { id, set } => {
            let i = s.components.iter().position(|c| c.id == id).ok_or_else(|| no_component(s, &id))?;
            let mut v = serde_json::to_value(&s.components[i]).unwrap();
            let fields: Vec<String> = set.keys().cloned().collect();
            for (k, val) in set {
                if val.is_null() {
                    v.as_object_mut().unwrap().remove(&k);
                } else {
                    v[k] = val;
                }
            }
            let c: Component = serde_json::from_value(v).map_err(|e| Reason::new("bad_field", e.to_string()).subject(&id))?;
            s.components[i] = c;
            Ok(format!("updated '{id}': {}", fields.join(", ")))
        }
        Command::RemoveComponent { id } => {
            let i = s.components.iter().position(|c| c.id == id).ok_or_else(|| no_component(s, &id))?;
            let was_box = s.components.remove(i).is_box();
            let mut moved = 0;
            for c in &mut s.components {
                c.goes_with.retain(|w| w != &id);
                if c.box_.as_deref() == Some(&id) {
                    c.box_ = None;
                    moved += 1;
                }
            }
            let first = s.boxes().first().map(|b| b.name.clone());
            Ok(match (was_box, first) {
                (true, Some(f)) if moved > 0 => format!("removed box '{id}'; {moved} components moved to {f}"),
                (true, None) => format!("removed box '{id}'; the game has no box now"),
                _ => format!("removed '{id}'"),
            })
        }
        Command::StoreAs { id, kind, set } => {
            let i = s.components.iter().position(|c| c.id == id).ok_or_else(|| no_component(s, &id))?;
            let (mut k, why) = crate::storage::derive(&s.components[i], &kind, &set)?;
            // A box marked again keeps what isn't given: its layout, tweaks, lid depth, top stack.
            if let (Kind::Box(new), Kind::Box(old)) = (&mut k, &s.components[i].kind) {
                if !set.contains_key("layout") {
                    new.layout = old.layout.clone();
                }
                if !set.contains_key("lid_depth") {
                    new.lid_depth = old.lid_depth;
                }
                if !set.contains_key("top_stack") {
                    new.top_stack = old.top_stack;
                }
                if !set.contains_key("lid_height") {
                    new.lid_height = old.lid_height;
                }
                if !set.contains_key("lid_lift") {
                    new.lid_lift = old.lid_lift;
                }
                if !set.contains_key("printed") {
                    new.printed = old.printed.clone();
                }
            }
            if kind == "box" && s.components[i].box_.is_some() {
                s.components[i].box_ = None;
            }
            s.components[i].kind = k;
            Ok(if why.is_empty() { format!("'{id}' is stored as {kind}") } else { format!("'{id}' is stored as {kind}: {}", why.join("; ")) })
        }
        Command::AddGroup { id, name } => {
            if s.groups.iter().any(|g| g.id == id) {
                return Err(Reason::new("exists", format!("group '{id}' exists")).subject(&id));
            }
            s.groups.push(Group { id: id.clone(), name });
            Ok(format!("added group '{id}'"))
        }
        Command::RemoveGroup { id } => {
            let before = s.groups.len();
            s.groups.retain(|g| g.id != id);
            if s.groups.len() == before {
                return Err(Reason::new("no_group", format!("no group '{id}'")).subject(&id));
            }
            for c in s.components.iter_mut().filter(|c| c.group.as_deref() == Some(&id)) {
                c.group = None;
            }
            Ok(format!("removed group '{id}'"))
        }
        Command::AddSleeve { sleeve } => {
            match s.sleeves.iter_mut().find(|x| x.id == sleeve.id) {
                Some(x) => *x = sleeve.clone(),
                None => s.sleeves.push(sleeve.clone()),
            }
            Ok(format!("sleeve profile '{}' is {} mm per card", sleeve.id, sleeve.per_card_mm))
        }
        Command::ChooseLayout { box_, strategy } => {
            if !layout::STRATEGIES.iter().any(|(id, _, _)| *id == strategy) {
                let ids: Vec<_> = layout::STRATEGIES.iter().map(|s| s.0).collect();
                return Err(Reason::new("unknown_strategy", format!("strategies: {}", ids.join(", "))).subject(&strategy));
            }
            let c = s.pick_box_mut(box_.as_deref())?;
            c.box_kind_mut().layout.strategy = strategy.clone();
            Ok(format!("{}: chose layout '{strategy}'", c.id))
        }
        Command::TweakTray { tray, name, notes, .. } if s.pieces.iter().any(|p| p.copies.iter().any(|c| crate::pieces::tray_id(p, c) == tray)) => {
            // An imported piece: only its name and the changes asked for it apply.
            let p = s.pieces.iter_mut().find(|p| p.copies.iter().any(|c| crate::pieces::tray_id(p, c) == tray)).unwrap();
            let mut what = vec![];
            let text = |v: String| Some(v.trim().to_string()).filter(|v| !v.is_empty());
            if let Some(v) = name {
                match text(v) {
                    Some(n) => {
                        if p.auto_name.is_none() {
                            p.auto_name = Some(p.name.clone());
                        }
                        p.name = n;
                    }
                    None => {
                        if let Some(a) = p.auto_name.take() {
                            p.name = a;
                        }
                    }
                }
                what.push(format!("name '{}'", p.name));
            }
            if let Some(v) = notes {
                spec::set_notes(&mut p.request, &mut p.log, &v);
                what.push("request".into());
            }
            Ok(format!("tweaked '{}' (imported){}", p.id, if what.is_empty() { String::new() } else { format!(": {}", what.join("; ")) }))
        }
        Command::TweakTray { box_, tray, max_depth, walls, finger_holes, open_ends, per_pocket, holders, grips, half_circle, scoops, color, label, pocket_labels, label_mode, order, pocket, notch, notch_keep, notch_floor, fill_spare, name, notes, unset, clear } => {
            // What changed, for the reply and the history.
            let mut what: Vec<String> = vec![];
            let mut said = |k: &str, v: Option<String>| if let Some(v) = v { what.push(format!("{k} {v}")) };
            said("max_depth", max_depth.map(|v| format!("{v} mm")));
            said("walls", walls.map(|v| format!("{v} mm")));
            said("finger_holes", finger_holes.map(|v| v.to_string()));
            said("open_ends", open_ends.map(|v| v.to_string()));
            said("per_pocket", per_pocket.map(|v| v.to_string()));
            said("holders", holders.map(|v| v.to_string()));
            said("grips", grips.map(|v| v.to_string()));
            said("half_circle", half_circle.as_ref().map(|v| if v.trim().is_empty() { "removed".into() } else { format!("'{}'", v.trim()) }));
            said("scoops", scoops.filter(|_| pocket.is_none()).map(|v| v.to_string()));
            said("color", color.clone());
            said("label", label.as_ref().map(|v| if v.trim().is_empty() { "removed".into() } else { format!("'{}'", v.trim()) }));
            said("pocket_labels", pocket_labels.map(|v| v.to_string()));
            said("label_mode", label_mode.map(|v| v.as_str().to_string()));
            said("order", order.as_ref().map(|v| if v.is_empty() { "back to the packer".into() } else { v.join(", ") }));
            said("name", name.as_ref().map(|v| if v.trim().is_empty() { "reset".into() } else { format!("'{}'", v.trim()) }));
            said("request", notes.as_ref().map(|v| if v.trim().is_empty() { "removed".into() } else { format!("'{}'", v.trim()) }));
            if !unset.is_empty() {
                what.push(format!("{} back to the packer", unset.join(", ")));
            }
            let what = if what.is_empty() { String::new() } else { format!(": {}", what.join("; ")) };
            let gb = s.pick_box(box_.as_deref())?;
            let current = layout::chosen(s, &gb);
            let whole = tray == spec::WHOLE_INSERT;
            if whole && (max_depth.is_some() || walls.is_some() || finger_holes.is_some() || open_ends.is_some() || per_pocket.is_some() || holders.is_some() || grips.is_some()
                || half_circle.is_some() || scoops.is_some() || color.is_some() || label.is_some() || pocket_labels.is_some() || label_mode.is_some() || order.is_some() || pocket.is_some() || fill_spare.is_some() || name.is_some() || !unset.is_empty() || clear)
            {
                return Err(Reason::new("whole_insert", "the whole insert ('*') takes only notes: settings go on a tray").subject(&tray));
            }
            if let Some(pc) = &pocket {
                // A frame's bays hand what they hold to their holders, so a frame takes its holders' too.
                let holds = current.trays.iter().any(|t| (t.id == tray || t.holder_of.as_deref() == Some(tray.as_str())) && t.pockets.iter().any(|p| &p.component == pc));
                if !holds {
                    return Err(Reason::new("no_pocket", format!("tray '{tray}' has no pocket holding '{pc}'")).subject(&tray));
                }
            }
            if !whole && !clear && !current.trays.iter().any(|t| t.id == tray) {
                let ids: Vec<_> = current.trays.iter().map(|t| t.id.as_str()).collect();
                return Err(Reason::new("no_tray", format!("no tray '{tray}' in {}'s '{}' layout (trays: {})", gb.id, gb.layout.strategy, ids.join(", "))).subject(&tray));
            }
            let layout = &mut s.pick_box_mut(box_.as_deref())?.box_kind_mut().layout;
            if let Some(c) = &color {
                if !(c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|x| x.is_ascii_hexdigit())) {
                    return Err(Reason::new("bad_color", "colour is #rrggbb").subject(&tray));
                }
            }
            if clear {
                // Settings go back to the packer; the request and the tray's log stay.
                let i = layout.tweaks.iter().position(|t| t.tray == tray);
                if let Some(t) = i.map(|i| &mut layout.tweaks[i]) {
                    *t = Tweak { tray: tray.clone(), request: t.request.take(), log: std::mem::take(&mut t.log), ..Default::default() };
                    if t.request.is_none() && t.log.is_empty() {
                        layout.tweaks.retain(|t| t.tray != tray);
                    }
                }
                return Ok(format!("cleared tweaks for '{tray}'"));
            }
            let i = match layout.tweaks.iter().position(|t| t.tray == tray) {
                Some(i) => i,
                None => {
                    layout.tweaks.push(Tweak { tray: tray.clone(), ..Default::default() });
                    layout.tweaks.len() - 1
                }
            };
            let t = &mut layout.tweaks[i];
            // One pocket's settings: scoops, a notch, and unsetting them.
            let mut pw: Vec<String> = vec![];
            if let Some(pc) = &pocket {
                let pt = t.pockets.entry(pc.clone()).or_default();
                if let Some(v) = scoops {
                    pt.scoop = Some(v);
                    pw.push(format!("{pc}: scoop {v}"));
                }
                if let Some(v) = fill_spare {
                    pt.fill_spare = Some(v).filter(|v| *v);
                    pw.push(format!("{pc}: {}", if v { "takes the spare room" } else { "leaves the spare room" }));
                }
                match notch {
                    Some(w) if w <= 0.0 => {
                        pt.notch = None;
                        pw.push(format!("{pc}: notch removed"));
                    }
                    Some(w) => {
                        let keep = notch_keep.or(pt.notch.map(|n| n.keep)).unwrap_or(5.0);
                        let floor = notch_floor.or(pt.notch.map(|n| n.floor)).unwrap_or(0.0);
                        pt.notch = Some(spec::Notch { width: w, keep, floor });
                        pw.push(format!("{pc}: {w} mm notch, {keep} mm of wall kept{}", if floor > 0.0 { format!(", {floor} mm into the floor") } else { String::new() }));
                    }
                    None => {
                        if let (Some(k), Some(n)) = (notch_keep, pt.notch.as_mut()) {
                            n.keep = k;
                            pw.push(format!("{pc}: notch keeps {k} mm"));
                        }
                        if let (Some(f), Some(n)) = (notch_floor, pt.notch.as_mut()) {
                            n.floor = f;
                            pw.push(format!("{pc}: notch {} the floor", if f > 0.0 { format!("bites {f} mm into") } else { "leaves".into() }));
                        }
                    }
                }
                if let Some(n) = pt.notch.filter(|n| n.floor > 0.0 && n.keep > 0.0) {
                    return Err(Reason::new("bad_notch", format!("{pc}: a notch can only go into the floor when it keeps no wall (notch_keep=0); it keeps {} mm", n.keep)).subject(&tray));
                }
                for f in &unset {
                    match f.as_str() {
                        "scoops" => pt.scoop = None,
                        "notch" => pt.notch = None,
                        "fill_spare" => pt.fill_spare = None,
                        other => return Err(Reason::new("bad_field", format!("can't unset '{other}' on a pocket: scoops, notch or fill_spare")).subject(&tray)),
                    }
                }
                if *pt == spec::PocketTweak::default() {
                    t.pockets.remove(pc);
                }
            }
            let (scoops, unset) = if pocket.is_some() { (None, vec![]) } else { (scoops, unset) };
            if max_depth.is_some() {
                t.max_depth = max_depth;
            }
            if walls.is_some() {
                t.walls = walls;
            }
            if finger_holes.is_some() {
                t.finger_holes = finger_holes;
            }
            if open_ends.is_some() {
                t.open_ends = open_ends;
            }
            if per_pocket.is_some() {
                t.per_pocket = per_pocket;
            }
            if holders.is_some() {
                t.holders = holders;
            }
            if grips.is_some() {
                t.grips = grips;
            }
            if let Some(v) = &half_circle {
                t.half_circle = Some(v.trim().to_string()).filter(|v| !v.is_empty());
            }
            if scoops.is_some() {
                t.scoops = scoops;
            }
            if color.is_some() {
                t.color = color;
            }
            if let Some(v) = &label {
                t.label = Some(v.trim().to_string()).filter(|v| !v.is_empty());
            }
            if pocket_labels.is_some() {
                t.pocket_labels = pocket_labels;
            }
            if label_mode.is_some() {
                t.label_mode = label_mode;
            }
            if let Some(v) = order {
                t.order = v;
            }
            for f in &unset {
                match f.as_str() {
                    "max_depth" => t.max_depth = None,
                    "walls" => t.walls = None,
                    "finger_holes" => t.finger_holes = None,
                    "open_ends" => t.open_ends = None,
                    "per_pocket" => t.per_pocket = None,
                    "holders" => t.holders = None,
                    "grips" => t.grips = None,
                    "half_circle" => t.half_circle = None,
                    "scoops" => t.scoops = None,
                    "color" => t.color = None,
                    "name" => t.name = None,
                    "label" => t.label = None,
                    "pocket_labels" => t.pocket_labels = None,
                    "label_mode" => t.label_mode = None,
                    "order" => t.order.clear(),
                    other => return Err(Reason::new("bad_field", format!("can't unset '{other}': max_depth, walls, finger_holes, open_ends, per_pocket, holders, grips, half_circle, scoops, color, name, label, pocket_labels, label_mode or order")).subject(&tray)),
                }
            }
            let text = |v: String| Some(v.trim().to_string()).filter(|v| !v.is_empty());
            if let Some(v) = name {
                t.name = text(v);
            }
            if let Some(v) = notes {
                spec::set_notes(&mut t.request, &mut t.log, &v);
            }
            // A tweak with nothing left in it goes.
            let empty = Tweak { tray: tray.clone(), ..Default::default() };
            if serde_json::to_value(&*t).ok() == serde_json::to_value(&empty).ok() {
                layout.tweaks.retain(|x| x.tray != tray);
                return Ok(format!("cleared tweaks for '{tray}'"));
            }
            let what = if pw.is_empty() { what } else if what.is_empty() { format!(": {}", pw.join("; ")) } else { format!("{what}; {}", pw.join("; ")) };
            Ok(format!("tweaked '{tray}'{what}"))
        }
        Command::SetCopies { piece, copies } => {
            let n = s.filament_slots().len();
            let p = s.pieces.iter_mut().find(|p| p.id == piece).ok_or_else(|| Reason::new("no_piece", format!("no imported piece '{piece}'")).subject(&piece))?;
            if copies.is_empty() {
                return Err(Reason::new("bad_copies", "a piece has at least one copy").subject(&piece));
            }
            let old = std::mem::take(&mut p.copies);
            for c in &copies {
                if c.filament.is_some_and(|f| f == 0 || f > n) {
                    return Err(Reason::new("bad_filament", format!("filament slots are 1 to {n}")).subject(&piece));
                }
                let kept = old.iter().find(|o| o.name == c.name).cloned().unwrap_or_default();
                p.copies.push(spec::PieceCopy { name: c.name.clone(), filament: c.filament, ..kept });
            }
            Ok(format!("{}: {} cop{}", p.name, p.copies.len(), if p.copies.len() == 1 { "y" } else { "ies" }))
        }
        Command::PlacePiece { piece, copy, at, turn, flip, tilt, inside, layer, box_, unplace } => {
            if let Some(b) = &box_ {
                if s.game_box(b).is_none() {
                    return Err(Reason::new("no_box", format!("no box '{b}'")).subject(b));
                }
            }
            if let Some(i) = inside.as_deref().filter(|i| !i.is_empty()) {
                let pid = i.split('/').next().unwrap_or_default();
                if pid == piece || !s.pieces.iter().any(|p| p.id == pid) {
                    return Err(Reason::new("no_piece", format!("can't go inside '{i}'")).subject(&piece));
                }
            }
            let p = s.pieces.iter_mut().find(|p| p.id == piece).ok_or_else(|| Reason::new("no_piece", format!("no imported piece '{piece}'")).subject(&piece))?;
            if box_.is_some() {
                p.box_ = box_;
            }
            let name = p.name.clone();
            let c = match &copy {
                Some(n) => p.copies.iter_mut().find(|c| c.name.as_deref() == Some(n.as_str())).ok_or_else(|| Reason::new("no_copy", format!("{name} has no copy '{n}'")).subject(&piece))?,
                None => p.copies.first_mut().ok_or_else(|| Reason::new("no_copy", "no copies"))?,
            };
            if unplace {
                c.at = None;
                return Ok(format!("{name} taken out of the box"));
            }
            if let Some(v) = tilt {
                if v % 90 != 0 {
                    return Err(Reason::new("bad_rotate", "tilt is 0, 90, 180 or 270").subject(&piece));
                }
                c.tilt = v % 360;
            }
            if turn.is_some_and(|t| t % 90 != 0) {
                return Err(Reason::new("bad_rotate", "turn is 0, 90, 180 or 270").subject(&piece));
            }
            if let Some(v) = at {
                c.at = Some(v);
            }
            if let Some(v) = turn {
                c.turn = v % 360;
            }
            if let Some(v) = flip {
                c.flip = v;
            }
            if let Some(v) = inside {
                c.inside = Some(v).filter(|v| !v.is_empty());
            }
            if layer.is_some() {
                c.layer = layer;
            }
            let at = c.at.map_or("unplaced".into(), |a| format!("at {:.1}, {:.1}, {:.1}", a[0], a[1], a[2]));
            Ok(format!("{name}{}: {at}{}", copy.map(|c| format!(" ({c})")).unwrap_or_default(), c.inside.as_ref().map(|i| format!(" inside {i}")).unwrap_or_default()))
        }
        Command::RemovePiece { piece } => {
            let before = s.pieces.len();
            s.pieces.retain(|p| p.id != piece);
            if s.pieces.len() == before {
                return Err(Reason::new("no_piece", format!("no imported piece '{piece}'")).subject(&piece));
            }
            s.plates.retain(|p| p.piece != format!("piece/{piece}"));
            Ok(format!("removed {piece}"))
        }
        Command::NamePlate { plate, name } => {
            if plate == 0 {
                return Err(Reason::new("bad_plate", "plates are numbered from 1"));
            }
            let name = name.trim().to_string();
            if s.plate_names.len() < plate {
                s.plate_names.resize(plate, String::new());
            }
            s.plate_names[plate - 1] = name.clone();
            while s.plate_names.last().is_some_and(|n| n.is_empty()) {
                s.plate_names.pop();
            }
            Ok(if name.is_empty() { format!("plate {plate} has no name") } else { format!("plate {plate} is '{name}'") })
        }
        Command::SetFilament { slot, preset, color } => {
            let mut slots = s.filament_slots();
            if slot == 0 || slot > slots.len() + 1 {
                return Err(Reason::new("bad_filament", format!("slot is 1 to {} ({} adds one)", slots.len() + 1, slots.len() + 1)));
            }
            if slot == slots.len() + 1 {
                slots.push(spec::default_filament());
            }
            let f = &mut slots[slot - 1];
            if let Some(p) = preset {
                f.preset = p;
            }
            if let Some(c) = color {
                f.color = c.to_uppercase();
            }
            let msg = format!("filament slot {slot}: {} {}", f.preset, f.color);
            s.filaments = slots;
            Ok(msg)
        }
        Command::SetLabels { mode, filament } => {
            if let Some(m) = mode {
                s.labels.mode = m;
            }
            if let Some(f) = filament {
                s.labels.filament = Some(f).filter(|&f| f != 1 && f != 0);
            }
            let slot = s.labels.filament.unwrap_or(1);
            let same = if slot == 1 && s.labels.mode == spec::LabelMode::Inlay { " (the trays' own slot: set_labels filament=N for a colour of their own)" } else { "" };
            Ok(format!("labels: {}, inlays on filament slot {slot}{same}", s.labels.mode.as_str()))
        }
        Command::RemoveFilament { slot } => {
            let mut slots = s.filament_slots();
            if slot == 0 || slot > slots.len() || slots.len() == 1 {
                return Err(Reason::new("bad_filament", "there's no such slot, or it's the only one"));
            }
            slots.remove(slot - 1);
            s.filaments = slots;
            let fix = |f: usize| if f == slot { 1 } else if f > slot { f - 1 } else { f };
            s.labels.filament = s.labels.filament.map(fix).filter(|&f| f != 1);
            for p in &mut s.plates {
                p.filament = p.filament.map(fix);
                for v in p.parts.values_mut() {
                    *v = fix(*v);
                }
            }
            Ok(format!("removed filament slot {slot}"))
        }
        Command::SetPlate { piece, plate, rotate, flip, filament, parts, unset, clear } => {
            if piece == "*" {
                if !clear {
                    return Err(Reason::new("bad_piece", "piece '*' only goes with clear=true"));
                }
                let n = s.plates.len();
                s.plates.clear();
                return Ok(format!("cleared {n} plate placements; the packer places everything"));
            }
            let pl = crate::plates::arrange(s);
            let Some(cur) = pl.pieces.iter().find(|p| p.id == piece) else {
                let ids: Vec<_> = pl.pieces.iter().map(|p| p.id.as_str()).collect();
                return Err(Reason::new("no_piece", format!("no piece '{piece}' (pieces: {})", ids.join(", "))).subject(&piece));
            };
            if plate.is_some_and(|n| n == 0 || n > pl.count + 1) {
                return Err(Reason::new("bad_plate", format!("plate is 1 to {} ({} is a new one)", pl.count + 1, pl.count + 1)).subject(&piece));
            }
            let name = cur.name.clone();
            if clear {
                s.plates.retain(|p| p.piece != piece);
                return Ok(format!("{name} goes back to the plate packer"));
            }
            let i = match s.plates.iter().position(|p| p.piece == piece) {
                Some(i) => i,
                None => {
                    s.plates.push(spec::PlatePin { piece: piece.clone(), ..Default::default() });
                    s.plates.len() - 1
                }
            };
            let p = &mut s.plates[i];
            let mut what = vec![];
            if let Some(n) = plate {
                if p.plate != Some(n) {
                    p.pos = None;
                }
                p.plate = Some(n);
                what.push(format!("plate {n}"));
            }
            if rotate.is_some() {
                p.pos = None;
            }
            if let Some(r) = rotate {
                p.rotate = Some(r % 360);
                what.push(format!("turned {}°", r % 360));
            }
            if let Some(f) = flip {
                p.flip = Some(f);
                what.push(if f { "upside down".into() } else { "the right way up".into() });
            }
            if let Some(f) = filament {
                p.filament = Some(f);
                what.push(format!("filament {f}"));
            }
            for (name, f) in parts {
                if !cur.parts.iter().any(|x| x.name == name) {
                    let names: Vec<_> = cur.parts.iter().map(|x| x.name.as_str()).collect();
                    return Err(Reason::new("no_part", format!("{} has no part '{name}' (parts: {})", cur.name, names.join(", "))).subject(&piece));
                }
                if f == 0 {
                    p.parts.remove(&name);
                    what.push(format!("{name} back to the piece's filament"));
                } else {
                    p.parts.insert(name.clone(), f);
                    what.push(format!("{name} on filament {f}"));
                }
            }
            for f in &unset {
                match f.as_str() {
                    "plate" => p.plate = None,
                    "rotate" => p.rotate = None,
                    "flip" => p.flip = None,
                    "pos" => p.pos = None,
                    "filament" => p.filament = None,
                    "parts" => p.parts.clear(),
                    other => return Err(Reason::new("bad_field", format!("can't unset '{other}': plate, rotate, flip, pos, filament or parts")).subject(&piece)),
                }
                what.push(format!("{f} back to the packer"));
            }
            if *p == (spec::PlatePin { piece: piece.clone(), ..Default::default() }) {
                s.plates.retain(|x| x.piece != piece);
            }
            Ok(format!("{name}: {}", if what.is_empty() { "unchanged".into() } else { what.join(", ") }))
        }
        _ => unreachable!("handled before apply"),
    }
}

/// A slug from a name: "Harbour Guild: Big Box" → "harbour-guild-big-box".
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    for ch in name.to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let mut out: String = out.trim_end_matches('-').chars().take(48).collect();
    while out.ends_with('-') {
        out.pop();
    }
    out
}

fn kind_name(c: &Component) -> &'static str {
    c.kind_name()
}

fn no_component(s: &Spec, id: &str) -> Reason {
    let ids: Vec<_> = s.components.iter().map(|c| c.id.as_str()).collect();
    Reason::new("no_component", format!("no component '{id}' (have: {})", ids.join(", "))).subject(id)
}
