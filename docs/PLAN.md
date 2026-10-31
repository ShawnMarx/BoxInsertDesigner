# BoxInsertDesigner — plan

A purpose-built tool for designing **3D-printed board game inserts**, driven by Claude, with
**SolidCraft** (tangency-labs/solidcraft, Rust parametric CAD) as the geometry backend.

It has two goals, and the plan serves both:

1. **The tool:** measure components by hand, let Claude propose box layouts, refine them, and
   print the result on a Bambu X1C.
2. **The thesis:** agent-built software becomes valuable as *customised versions for particular
   purposes*. This project tests whether a purpose-built version can keep up with an upstream
   that releases every day (~800 commits a week at the time of writing). It measures what that
   upkeep costs, without assuming the answer.

Started 2026-10-27. Built for Shawn's own use; the repo is private, and it may be shared later.

---

## Architecture (decided 2026-10-27)

```
iPad / laptop browser
  └─ web front end (web/) — components, layout options, 3D view (three.js + glTF),
     fit display, layer slider, chat, export
          │ a private network only (e.g. tailscale serve → 127.0.0.1)
host      ▼
  └─ insert server (crates/server, Rust)
       spec in git · layout packer · print rules · Bambu 3MF + bulk STL writer
       MCP tools for Claude
          │ SolidCraft commands (JSON, the same contract its MCP and scripts use)
          ▼
     SolidCraft, pinned release, headless — builds geometry, measures, exports meshes
```

- **The front end is our own web app.** SolidCraft's UI is not the front end. For the occasional
  hand edit, open the generated design in SolidCraft itself (desktop or web).
- **The spec is the master copy.** One file per game, in git. Geometry, meshes and 3MFs are all
  generated from it. Browser sessions are views of it.
- **Claude decides intent; the engine does the maths.** Claude proposes and refines layouts
  through typed tools. A deterministic packer places them and checks they fit. Every derived
  number can explain where it came from.
- **One command surface.** Every action is a typed command, used by both the MCP tools and the
  UI. Nothing works only in the UI or only for Claude.
- **Output files are produced on the server**, never downloaded from a browser tab.

### How SolidCraft changes are handled (in order)

1. Do it in our own server.
2. Build it from existing SolidCraft commands.
3. Upstream PR. Once merged, it costs nothing to carry.
4. **Tier C fork patch**, only when a goal needs it. New code goes in new files, with one-line
   hooks into upstream files, and each patch gets an entry in `docs/PATCHES.md`.

Tier C is allowed, not avoided. Every entry in `docs/PATCHES.md` records what it costs to carry
across releases, and those records are the evidence for goal 2.

---

## Milestones

### M0 — Scaffold ✅ (2026-10-27)
- [x] Repo, plan, CLAUDE.md, private GitHub remote
- [x] Rust server skeleton serving `web/` and `/health` on 127.0.0.1:8102
- [x] LaunchAgent (`launchd/`), published on a private network only (`tailscale serve`)

### M1 — Upstream harness (the thesis needs this first)
- [x] Pin a SolidCraft release (`upstream.toml`: tag + SHA-256 from its `SHA256SUMS.txt`) —
      v0.24.1, 2026-10-27
- [x] Fetch the release `solidcraft-cli` for macOS; no compile needed for the headless backend
      (`tools/fetch-solidcraft.sh` checks the zip against the pin and `SHA256SUMS.txt`)
- [x] **Command contract test:** record each command we use with its input/output schema from
      `solidcraft-cli commands`, and diff against every new release. `tools/contract/check.py`
      records the params text of the 6 commands we call (`used.txt`) and diffs a CLI against it.
      Output schemas aren't recorded yet (the catalogue only describes them in prose).
- [x] `docs/PATCHES.md` patch list, even while it's empty
- [x] Daily upkeep job (2026-10-27): new release tag → verify SHA256 → contract test →
      build → test → smoke build of every game → bump and deploy on green, or start a fix
      session (`tools/upkeep/SESSION.md`) that fixes, commits and lands it. LaunchAgent
      a daily LaunchAgent (`launchd/`), record in `docs/UPKEEP.md`.
      Re-applying patches comes in once there are any.

### M2 — The spec
- [x] Serde types and validation (`crates/server/src/spec.rs`, TOML at `games/<slug>/spec.toml`) for:
  - [x] box: inner size, lid depth, top stack — [ ] storage (flat or vertical)
  - [x] several boxes per project, each with its own layout and tweaks; a component's `box`,
        default the first. **A box is a component** (`kind = "box"`: inside, lid depth, top
        stack, layout), so it has photos, notes and an outside `measure` like any other, and its
        faces are `lid`, `front` (a long side) and `side` (a short side). Older `[box]` and
        `[[boxes]]` specs load and are rewritten. Boxes don't go inside other boxes (decided
        2026-10-27: not needed)
  - [x] measurements as taken (`measure`): one piece and/or a stack (with its count), the count
        in the game, outline, extra labelled measurements; `notes` on how it goes in;
        `goes_with`; photo notes. Storage is `undecided` until picked; `as_is` goes in the box
        with no insert. `store_as` derives the packer's fields from the measurements, with a
        `why` per field, or names what's missing (2026-10-27)
  - components:
    - [x] card deck: measured stack *or* count + sleeve profile (stored flat only, for now)
    - [x] mini: base and height (one round pocket each)
    - [x] tokens: shape, size, thickness and count, in stacks (hex held in its bounding rectangle)
    - [x] standard bins
    - [ ] exact outline: SVG or STL
  - [x] groups — [ ] access order, at-table use
  - [x] sleeve profiles (two estimated defaults; measured per brand later)
  - [x] printer profile: X1C, 0.4 nozzle, 256×256 bed; clearances flagged UNMEASURED until tested
  - [x] label mode (2026-10-29): `[labels] mode = inlay | cut` and the inlays' `filament` slot
        (`set_labels`); a tray's `label_mode` tweak overrides it
- [x] Provenance on every derived dimension (each pocket, tray and fit number carries a `why`)
- [ ] One real game measured end to end as the reference fixture

### M3 — Geometry pipeline
- [x] Spec → SolidCraft command script → run headless → glTF (2026-10-27). v0.24.1 can't export
      glTF, so bodies come back as STL and our server packs them into a GLB (open question below).
      Measurements: not read back yet; the packer's numbers are what the UI shows.
- [ ] **Spike: subprocess vs embedded.** Subprocess is in use and fast enough (RESEARCH.md,
      experiment 3); embedded not measured. Run `solidcraft-cli` as a subprocess (it depends only on
      the command contract), or depend on the `solidcraft-engine` crate pinned to a tag (in
      process, but coupled to Rust APIs, with a heavy compile). Measure both and record the
      decision.
- [x] Ghosted component bodies and trays, bodies named after tray and pocket ids
- [x] Labels (2026-10-29): see Labels — decisions below. [ ] Lids, named folders

### M4 — Web front end v1
- [x] Two tabs per project (2026-10-27): **Components** (list + form: measure, photos with
      notes, how it goes in, used with, optional "stored as", box) and **Insert**. "New project"
      asks only for the name and opens on Components; the box is added when measured
- [x] Box art: an optional "Box art" toggle draws a plain base and the telescoping lid lifted
      above it, the lid's photos on its top and skirt; "Alone" shows the closed box, its lid
      riding at the layout's lid lift
- [x] The insert as a tree: layer → tray → the game components it holds → their pockets (with
      each number's why), then what has no tray. A tray can be viewed **alone** or **in its box**, with or
      without the components inside it. Trays can be **renamed**, and carry **notes for Claude**
      (the changes wanted); both are tray tweaks, and neither rebuilds the geometry
- [x] **Tray tab** (2026-10-27): "Open" on a tray gives it a tab of its own: the tray alone in 3D
      (with or without what it holds), its name and changes for Claude, its settings (deepest
      pocket, walls, finger holes, scoops, colour; empty = the packer decides), what it holds
      with each pocket's why, and a picker to step through the trays. `?tab=tray&tray=<id>`
- [x] **Save versioning** (2026-10-27): `games/` is its own git repository, versioned by the
      server: a game is committed 30 s after its changes settle (message = the commands behind
      them, and "spec edited by hand" for hand edits); named versions are tags; restore is a
      new commit; pushes to its `origin` when it has one (for a private GitHub repo, through gh's credential helper). The app has History (list, save a version,
      restore) and a "Saving… / All changes saved" indicator. The code repo no longer tracks
      game data, so edits in the app don't block upkeep's redeploy
- [x] Tray history (2026-10-29, Shawn): "Mark read" on an answered request clears the note and
      reply from the tray into its log (`mark_read`); changes asked for directly in a session are
      logged with `log_ask`; a new note never discards an answered one. The Tray tab's History
      (`tray_history`) lists them with the saved changes whose commands name the tray, its frame
      or what it holds, newest first
- [x] Requests queue (2026-10-27): each tray note is a request (id, status open → working →
      done/declined, reply). Agents poll from outside: `requests`, `wait_requests` (long-poll,
      returns `now_ms` for the next call) and `update_request`, all in `bid mcp`. The app shows
      status and reply on the tray. Brief: `tools/requests/AGENT.md`. Run on demand from a session
      (MCP registered user-scope); [ ] a standing agent on the server, if on-demand proves out
- [x] Insert tab: box picker when there are several, "Where everything goes" (in the insert /
      as-is / not decided / no place yet), each component viewable alone (from its
      measurements, with its photo) or picked out in its box; as-is items as see-through slabs
      in the top stack, with a warning when they're thicker than the top stack
- [x] Component photos (iPad camera or library through the web app, or `bid add_photo`) →
      four corners marked per face → texture straightened to the measured proportions on the server
      → textured placeholders at the ghost's exact size: card art on decks, textured tokens and
      bins, a camera-facing photo cut-out on a base for minis (2026-10-27)
- [x] glTF viewer (three.js 0.170.0): see-through box with the top-stack band, ghosted
      contents, tray colours, layer slider, "lift layers"; live over SSE, iPad and phone layouts
- [ ] Section view, lift-out order
- [x] Component list with measurements; fit report (clearance under the lid, bed fit, tray count)
- [ ] Filament estimate in the fit report
- [x] Tapping a body selects its tray

### M5 — Layout engine
- [x] Deterministic packer: trays, pockets, stacks, under-lid check, bed fit, walls and floors
      in whole lines and layers (`crates/server/src/layout.rs`)
- [x] Layout options built from strategies (by-group / by-kind / fewest-trays), compared side by
      side. [ ] Card rail and upright card storage
- [x] Tweaks as spec diffs (`[[layout.tweaks]]`). Live builds rebuild only changed trays;
      [ ] export still writes every tray

### M6 — Claude, first class
- [x] MCP server exposing the same commands as the front end: `bid mcp` (stdio, hand-rolled
      JSON-RPC, no `rmcp`), forwarding to `POST /api/command`. Not registered in the repo: a
      checked-in `.mcp.json` made every new session (and the unattended upkeep session) stop
      at "New MCP server found", so it is opt-in per user (CLAUDE.md)
- [ ] Screenshots or renders Claude can look at, plus the structured numeric reports
- [ ] `insert-design` skill: Shawn's conventions, and lessons as they're learned (e.g. the TI4
      3-ship base fix)
- [ ] Chat inside the web app (Remote Control first; an embedded pane later)
- [x] The app's own agent (2026-10-30, Shawn): with a Claude API key in Settings, the server works
      open requests itself (Messages API, tool use over the command surface, the request's game
      only; model and thinking level chosen there, default Claude Opus 5.5 at high). An outside
      agent still works: the corner chip says which is watching. [ ] First real run with a key

### M7 — Print output
- [x] Printability: style-guide walls and floors (normal → minimum only to fit, reinforced for
      heavy cards), tray heights in whole layers, and a mesh check that nothing needs supports
      (no overhang flatter than 45°)
- [x] Printed boxes with a slide-on, click-shut lid, sized to their insert (see below)
- [x] Card holders (2026-10-27, from Shawn's Forest Shuffle reference): tray tweaks `per_pocket`
  (one tray per deck, so each lifts out alone; like-sized trays share a shelf) and `open_ends`
  (no short end walls, a floor window in place of the finger hole)
- [x] Holder frames (2026-10-27, Shawn: usability first, decks must lift out): tray tweak
  `holders` builds a floorless frame with a lift-out card holder in each bay (walls.min walls
  on all four sides since 2026-10-28, a floor window, 0.3 mm to the bay) and spends the layer's spare room on finger
  slots between bays, cut down to a low bar. No added height: holders stand on the box floor
- [x] Motion video (2026-10-27, Shawn): the viewer's Motion button plays the box opening, each
  piece going out to the table one at a time (lid, what lies on top, then trays top layer first,
  holders before their frame), and packing back. Options per viewer in localStorage. Export
  records the canvas in real time (MediaRecorder: mp4 where the browser offers it, else webm).
  Front end only (`web/motion.js`); it moves the viewer's meshes and puts them back. On the
  Tray tab it plays just that tray's part (a frame's: its holders), with what came out before
  it already on the table, and holds where it lands (2026-10-28)
- [x] Measure on the Tray tab (2026-10-28, Shawn): with Measure on, a tap near an edge lights
  the whole straight edge and gives its length; two taps give the distance between two points
  (snapping to corners), with its x/y/z parts. Read off the built mesh (`web/measure.js`). With
  Cut view on it measures what the cut leaves: taps pass through the cut-away part, land on the
  cut face, and snap to its outline and corners
- [x] Cut view on both tabs (2026-10-28, Shawn): Horizontal sweeps a vertical section plane
  along the longer side (keeping the far side, so the cut faces the camera), Vertical lowers a
  horizontal one from the top; each has a swap toggle. Solids are cut solid: stencil caps in
  each mesh's colour (`web/cut.js`; the renderer needs `stencil: true`, off by default in three r163+)
- [x] Grips from Shawn's Anachrony trays (2026-10-27): tray tweak `half_circle` (a 25 mm
  half-circle notch in the long side facing the next tray, centred or just past a component's
  pocket; through the floor, except over loose pieces, where a 0.4 mm sill keeps them in) and
  `grips` on a frame (a 30 × 4 mm 45° lip on each holder's long wall, reaching into the finger
  slot, staggered so facing lips miss). Sizes in the style guide's `[grips]`, Shawn's values
- [x] Grab holes are a standard (Shawn, 2026-10-28): `half_circle` is now a **pair** of 25 mm
  half-circles facing each other across the tray (close: a pinch; far apart: two hands), top to
  bottom, in a solid band between pockets, never into one. The packer makes the band from the
  room stretching leaves, moving the later pockets along; in a tray of loose pieces the rest of
  that room goes to the pockets instead of an empty spare. The room across the tray does too;
  in other trays its spare stops either side of the holes rather than running through them. Every
  tray floor is 2 mm (style `floor.normal = floor.min = 2`).
- [ ] Test-print the Palace Heist card holders: four 1.6 mm walls, 2 mm floor with the window, 0.3 mm to the bay
- [ ] Test-print a printed box: slide gap, detent and groove values are proposals
- [ ] Chamfer on bottom edges, magnet holes with the layer number to pause at
- [ ] Bed fit: one piece first, then diagonal placement, then a shorter part, and **only then**
      a split (along a wall, with pins or a dovetail, and matching marks)
- [x] **Bambu project 3MF** (core): plates, named objects, orientation, filament slots. Copy
      the structure of a project saved in Bambu Studio. Shawn, 2026-10-28: design around Bambu,
      starting with his X1C, with a **Plate view tab** for arranging trays on plates before
      export (normally the slicer's job). Next session's lead item.
      Scoped with Shawn 2026-10-28: **auto-pack** trays onto 256 mm X1C plates, then edit in the
      Plate tab (move to another plate, rotate); write a **project 3MF with plates and the X1C
      0.4 / 0.20mm Standard preset** that opens in Bambu Studio for slicing; **no AMS slots yet**
      (all on slot 1 until multicolour labels); orientation **as it sits in the box** (lid of a
      printed box top face down), with a per-piece flip/rotate, rechecked for supports. Format
      reference: Shawn's own Anachrony project (X1C, Studio-saved);
      `ForestShuffle-MW.3mf` (P1S, 3 plates) for multi-plate structure only.
      Built 2026-10-28: `plates.rs` packs every piece (trays, holders, a printed box and lid),
      largest first, 5 mm apart, inside the 3 mm margin and clear of the X1C's excluded
      18 × 28 mm front-left corner (`bed_exclude_area`, now `printer.bed_exclude`); `set_plate`
      pins a piece's plate, turn or flip (`[[plates]]` in the spec); a flipped piece is rechecked
      for supports. Export writes `<slug>-plates.3mf` (`bambu.rs`): plates 1.2 beds apart in rows
      of ⌈√n⌉ as Studio lays them out, meshes centred per object, Studio's assembly view showing
      the pieces in the box, and Shawn's X1C settings (`assets/bambu/`). Checked structurally
      against the reference; **not yet opened in Bambu Studio** (not installed on the server)
- [ ] Open the first `<slug>-plates.3mf` in Bambu Studio and slice it (Shawn): does it load as
      a project with its plates and the X1C preset?
- [ ] Packer bed cap vs the excluded corner: the layout packer allows trays up to 250 × 250, but
      a tray over 238 × 228 can't miss the X1C's excluded corner (the sample's 250 × 250
      "Shared" tray). The plate packer reports it; should the layout packer cap to fit?
- [x] Plates, round 2 (Shawn, 2026-10-28): **named plates**; pinning a piece to a plate keeps
      auto-arranging on it, with an error when it overflows; **filament slots** set in the Plates
      tab (slot 1 default Bambu PLA Matte black), a slot per piece, and a slot per **named part**
      of a piece (a label line, a logo), as Studio stores parts; colour the Plates view by app
      colour or by filament. Filament settings come from Shawn's own Studio projects.
      Built: the project's settings are cut slot by slot from Shawn's 10-slot Anachrony project
      (`assets/bambu/`; presets PLA Matte, Galaxy, Sparkle, Metal). Cut to its black Matte slot it
      gives his one-slot project back but for `filament_colour_type`, so the method holds; a
      preset he hasn't used in a saved project has no settings yet
- [x] **Import as-is pieces** (Shawn, 2026-10-28): a new project from his full Anachrony build
      (`Anachrony/Anachrony.3mf`: 43 objects, multi-part labels, 10 filament slots, 22 named
      plates), pieces kept exactly as they are, the plates kept as they are, and placed in the box
      from his guide (`Anachrony/Photos/Box Top Down/Anachrony Insert - Guide.pdf`, 8 layers).
      Pieces that go inside others (player trays in player boxes) are placed inside them. Box
      inside inferred from the trays. Placement of imported pieces is explicit (x, y, z, turn),
      since the packer doesn't make them.
      Built (`pieces.rs`, game `anachrony`): all 43 objects, 22 plates exactly as in his file, 10
      slots; the player trays ×5 (×4 for the side-rail exo tray) as copies in each faction's slot,
      placed inside their player boxes, and the solo trays inside the Solo box. The box inside,
      288.5 × 419 × 226 mm, is inferred: the floor plan closes to the millimetre (2 × 20.8 card
      holders = Doomsday's 41.6; 162 + 125 across the top row; 139 + 148 player boxes; 3 × 139 deep).
      `tools/imports/anachrony-place.py` records how. **Approximate, for Shawn to check**: the
      order of trays inside each player box, which way up the player boxes' openings face, the
      Buildings lid (assumed on the tray, not upside down), heights from layer 3 up (from the
      trays under them, not measured), and where the solo inner tray and flux cores sit
- [x] Imported pieces are trays in the app (Shawn, 2026-10-28: "have the tray pieces in the app
      fully"): tree, 3D selection, Tray tab with Measure/Cut/Motion, renames and requests for
      Claude. Contents and boards from his photos (`tools/imports/anachrony-contents.json`, read
      from the guide photos and rulebooks by an agent): 63 entries kept in their trays (`piece`),
      14 boards lying in the box at their guide layer (`place`). All estimates, each with its
      evidence in its notes. Least certain: the Impact board (in no rulebook list), the order of
      the layer 5 module boards, the folded main board's thickness and the Solo board. Heights
      fitted to them: scorepad at 45 (5 miniboards at ~1 mm), hypersync and quantum loop at 64
- [x] Photos and faces for the Anachrony contents and boards (Shawn, 2026-10-28): each of the 77
      has a crop of the guide photo it shows best in as its photo (`games/anachrony/photos/`,
      5.2 MB), and 53 have their top face marked (`tools/imports/anachrony-faces.json`, picked by
      an agent; spot-checked). No face: closed kits, cards standing on edge, and what isn't
      visible (Adventure, Hypersync and Solo boards among them). Contents kept in an imported tray
      have no pocket positions, so their faces show in the Alone view and the Components tab, not
      in the tray; boards show theirs in the box
- [x] Contents in their trays' compartments (Shawn, 2026-10-28: "I do have a photo top down of
      every tray, with components"; "enough for top down visual"): `detect_pockets` finds each
      imported tray's compartments in its mesh (123 across the 43 pieces); an agent matched the
      components to them from his breakdown photos (`Anachrony/Photos/*.jpg`,
      `tools/imports/anachrony-pockets.json`), broke the six kits into 24 components, and picked
      31 faces. 78 of 81 contents sit in a pocket, drawn as textured ghosts (a stack too tall for
      its pocket fills it as a pile). The photos corrected the list: base-game Anomaly tiles are in
      Buildings, the Solo Hypersync tile in the solo box, the warp device in its path and warp
      tray, the solo flux cores and inner tray inside their parents. Multi-level trays (FoT setup,
      the side-rail exo tray) came out as one pocket each
- [x] A tray "In box" without the clutter (Shawn, 2026-10-28: a pulse, then fade the items
      around the tray, leaving it and the box frame; a toggle to keep a frame of what's hidden,
      which applies to the layer slider too). Built: the view's **Around** toggle — **Pulse**
      (what's around fades at once, comes back once, then stays away; once gone, each move or zoom pulses it in and out
      again), **Frame** (each piece's outline), **Shade** (the original look: left faintly there) or
      **Hide**. Layers above the slider are
      treated the same. "In box" now frames the tray itself, from its meshes, so trays inside a
      player box no longer open small and off-centre
- [ ] The packer and imported pieces: nothing stops the packer putting a generated tray where an
      imported piece is (the Anachrony box has no generated trays, so it doesn't arise yet)
- [x] **Bulk STL**: one per tray into `games/<slug>/out/`, plus a generic 3MF of the whole insert
- [x] One STL per colour for multicolour labels: export writes `<slug>-<tray>-label.stl` beside
      each inlaid tray (2026-10-29)
- [ ] Drawers and slide-to-click lids, when a game calls for them (Shawn's Anachrony drawers
      as the reference); everything else stays lidless
- [x] Labels: multicolour (a flush separate body with a filament slot) or single colour (cut)
      (2026-10-29, Lorcana the test game; see Labels — decisions). Embossed (standing proud) not
      built: it would need supports or a flat top layer over it, and Shawn's Anachrony labels are
      all flush inlays
- [ ] Test-print a labelled tray (Lorcana Tokens: inlaid, white on brown): do 0.6 mm
      deep, 4–8 mm capitals in Liberation Sans read, and does Studio print the inlay as one object
      of two parts without gaps? `labels.min_height` and `labels.margin` are proposals
- [ ] Labels on walls (vertical faces) and in a font other than SolidCraft's built-in one
- [x] **Edit in SolidCraft** (Shawn, 2026-10-30; built 2026-10-30): a button on an opened tray
      (Tray tab) opens SolidCraft's web app in a new tab holding that tray, with its full history
      to edit by hand. The public solidcraft.io/app can't be pointed at a file and its
      `window.solidcraft` API is in-page only (SolidCraft `docs/browser-api.md`), so the server
      serves the release's `solidcraft-web-<ver>.zip` at the pinned tag on `/solidcraft/`
      (gzipped: the wasm is 94 MB raw, 30 MB gzipped), fetched with the CLI by
      `tools/fetch-solidcraft.sh` and by upkeep (`web_asset`, `web_sha256` in `upstream.toml`).
      `web/solidcraft.js` replays the tray's script (`edit_script`: the tray alone in print pose)
      with `engine.batch`, then follows `document.revision`. When the edits pause it saves the
      design with `file.save_as` to `/opfs/…`, reads it back on the same origin (OPFS, or
      localStorage hex where the page isn't https), and sends it with `submit_edit`.
      **Observed, not taken:** `edit.rs` keeps it as `games/<slug>/edits/<tray>.sc` and compares
      it with the generated tray (`solidcraft-cli script` gives the timeline by feature id,
      `eval` gives volumes and sizes, `snapshot` draws both). The Tray tab shows the comparison
      and can put it into a note for the agent, which reads it with `edit_diff`.
      Over plain http (no HTTPS certificates on the private network) the app runs on WebGL2 and its
      own files go to localStorage. A WebGPU start that stalls for 45 s retries on WebGL2.
      Tested end to end in headless Chromium against a scratch server, not yet by Shawn.
- [x] SolidCraft workflows, all three (Shawn, 2026-10-30): (1) **note**: the edit is compared
      and sent to the agent as a note, so it lasts through later changes (the default; how the
      2 mm corners came about). (2) **keep as the tray** (`keep_edit` / `drop_edit`): the tray
      prints as the edit, a frozen copy in `edits/<tray>.kept.sc`; when the generated tray
      changes, the edit's added features (`<tray>.kept.json`, timeline steps) go on the new
      one, or it falls back to generated with a warning; changes to generated features don't
      carry over. Reopening it loads the kept design (its history as steps from
      `solidcraft-cli script`). The insert-wide 3MF still shows it as generated. (3) **piece
      from scratch** (Plates tab, `import_design` / `piece_design`): each save makes or updates
      a piece, each body a part, the design kept as `pieces/<id>/design.sc`
- [ ] SolidCraft workflows, gaps: the box-wide `insert.3mf` shows a kept tray as generated
      (the plates project and STLs are right); a new piece is placed in the box only with
      `place_piece` (no button yet); Bambu Galaxy Green `#3B665E` unchecked in
      `assets/bambu/colours.json`
- [ ] Edit in SolidCraft, next: Shawn's first real edit; HTTPS on the private network (for
      Tailscale: enable its certificates, then `tailscale serve --https`) for WebGPU
      and OPFS; whether a sketch's own curves (the label text) should be compared, since
      `script` gives sketches as features without their geometry
- [x] Plates keep clear of the front and place the purge tower (2026-10-30, Shawn: Bambu Studio
      warned "Decks is too close to exclusion area"): pieces keep `EXCLUDE_CLEAR` (5 mm) from
      the excluded corner (never over it), and stay out of the strip along the front beside it
      when any plate has room: the X1C draws its calibration lines there, so it's a preference,
      and the strip is used when nothing else fits. Every plate keeps its back-right corner free for the purge tower (`TOWER`
      41 × 46 mm with brim, an estimate of Studio's tower). A plate printing more than one
      filament gets the tower there, written into the project as that plate's
      `wipe_tower_x/y`; the Plates tab shows it
- [x] Card-draw slots on holders (2026-10-30, Shawn on Lorcana's deck holders): a notch set on a
      frame's bay (`tweak_tray tray=<frame> pocket=<component> notch=30 notch_keep=0
      notch_floor=15`) is its holder's. With `notch_floor` the slot goes on down through the
      floor and ends in a half-circle bite, as on Anachrony's pioneer tray, in place of the floor
      window. A notched holder's lip moves to an end wall. The bite is 0.25 mm a side narrower
      than the slot: as wide, or elliptical, SolidCraft v0.27.1 leaves open edges
- [x] Per-bin settings (2026-10-30, Shawn on Lorcana's Health dials): `tweak_tray pocket=<component>`
      sets that bin's own scoop, and a notch: a slot through both long walls at its middle, from
      the top down to `notch_keep` mm above its floor (to pinch out dials standing on edge)
- [x] Bins widen to their row (2026-10-29, Shawn on Lorcana's -1 tokens: "wider is better as it
      is easier to grab"): a bin narrower than its neighbours takes the room beside it, up to the
      next pocket, the wall or the grab holes, and is made shallower to hold the same. Tray tweak
      `order` sets the pockets' order along a tray

### M8 — Test fit
- [ ] Clearance test piece and gauge plates (card stack, token holes) as generated prints
- [ ] Fit slice: a 2–3 mm tall print of just the pocket outlines
- [ ] Measured results flow back into the printer profile

### M9 — Upkeep verdict
- [ ] After several weeks of daily runs (`docs/UPKEEP.md`), write up the carrying cost per patch, the time
      spent on conflicts, and contract breaks. Decide whether purpose-built versions are viable.

### M10 — Public release
Scoped with Shawn 2026-10-30. The pitch: anyone can run their own agent-driven insert design
process from a phone or the web. Measure and photograph components, work through layout
variations and refinements with an agent, review the finished insert in 3D, and edit in
SolidCraft with changes going back and forth between the agent and the app. Decided:
- **A fresh public repo with one starting commit.** The current GitHub repo is renamed
  `BoxInsertDesigner-archive` (private, archived), and all work moves to the public one.
  `~/repos/BoxInsertDesigner` keeps its path; its remote and main are switched over, and the
  upkeep worktree is recreated.
- **No personal setup in the public repo:** no host names, private-network addresses, service labels or personal paths.
  The private games repo is the personal workspace, holding a `config.toml` (bind, port, folders,
  SolidCraft path; the env vars still override it), the launchd plists, a personal `CLAUDE.md`
  (loaded through a gitignored `CLAUDE.local.md`) and the per-game import scripts. The public
  repo ships `config.example.toml` and template plists.
- Public commits keep `Co-Authored-By` and drop the private `Claude-Session` link.

- [x] LICENSE-MIT + LICENSE-APACHE (Cargo.toml already declares `MIT OR Apache-2.0`)
- [x] Config file replacing the env-var defaults; `upkeep.sh` reads service label and health URL from it
- [x] Move personal setup to the games repo; make CLAUDE.md, README and docs generic
- [x] Note where `assets/bambu/` came from: Bambu's stock X1C presets, as saved in a Studio project 3MF
- [x] Host-header check on the server (stops DNS rebinding against the loopback service):
      loopback, the bind address and `[server] hosts`; anything else gets 421
- [x] Photo metadata: every upload is decoded and re-encoded (`save_photo`), so none is kept
      on any path; now the EXIF orientation is applied first, so a phone photo sent with
      `add_photo file=` stays upright (test: `photo_is_turned_upright_and_stripped`)
- [x] README written around the pitch above
- [x] An example game: the Lorcana insert, to go with a MakerWorld post. Its card, board and
      box art blurred beyond recognition (marked faces automatically, the rest by hand), with
      faces regenerated from the blurred photos; Shawn reviews them before anything is public.
      Done 2026-10-30: `examples/ccg-example/`, the art pixelated 56 blocks across (decks, rules
      and villain cards, board, box; tokens, dials and the stand as taken), Lorcana's name out
- [x] Switch over: archive the old repo, push the fresh one public, repoint the local clone.
      Done 2026-10-30: public from `2929246`; old history in `BoxInsertDesigner-archive` (private,
      archived; the local remote `archive`)

### M11 — Beyond the Mac, and being seen
- [x] Windows (2026-10-30): builds on SolidCraft's portable Windows package (`windows_*` pins in
      upstream.toml, moved by upkeep), `tools/fetch-solidcraft.ps1`, agent settings in
      `%APPDATA%`, verbatim `\\?\` paths stripped. Run end to end on Windows 11 (41/41
      tests, ccg-example exported, Edit in SolidCraft); `docs/WINDOWS.md`. Not on Windows yet:
      running as a service, daily upkeep
- [ ] Linux: same shape (SolidCraft's Linux packages probably carry the CLI; unchecked)
- [ ] SolidCraft visibility. Done 2026-10-30: README "Built on SolidCraft" and the independence
      line (their DISCLAIMER.md), GitHub topics. 2026-10-31: showcase PR
      tangency-labs/solidcraft#10095 (GIF from ccg-example's Motion video, pixelated art), open;
      the Discord post is Shawn's to send (text in the games repo)
- [x] MakerWorld post for the Lorcana insert (2026-10-31), linked from the README; its link goes
      in BGE's `BOX_INSERT_DESIGNER_MAKERWORLD_URL`. The BGE project page is on staging; prod is
      on Shawn's go in BGE

---

## Insert conventions (2026-10-27)

These live in `styles/default.toml`, the build style guide, which the packer and the checks
read. Values are in mm; anything still in the file's `proposed` list is Claude's guess.

- **Tolerances are minimums, per side:** print ↔ measured component **0.8**. That covers pockets,
  headroom above contents, tray ↔ box wall, and the gap under the box lid. Print ↔ print **0.1**
  (tray ↔ tray). A component may ask for more (`clearance`), never less.
- **Walls:** aim for **2.0**. Drop to **1.6** only where that makes the insert fit. **1.2** only
  in small areas supported by the rest of the print, so it is never automatic (a `walls` tray
  tweak). Up to **3.0** for outer walls holding heavy card stacks.
- **Rounded corners** (2026-10-30, from Shawn's first edit in SolidCraft): every tray's four
  outer vertical corners get a **2.0** fillet (`walls.corner_radius`). Over a 1.6 wall that
  leaves 1.43 round a pocket's square corner; the radius shrinks only where it would leave less
  than `walls.absolute_min`.
- **No supports:** no overhang flatter than **45°** from horizontal. Every exported mesh is
  checked.
- **Game boxes have telescoping lids.** A box records the lid's height (how far it telescopes;
  unset = the full depth) and, as a design choice, an allowed **lid lift**: how far the closed
  lid may ride above the rim when components stand proud of it. The packer packs under a closed
  lid first and uses only the lift it needs, in whole layers; the fit report says how much.
- **No lids by default.** Exact fit plus the box lid hold everything in place, so the top layer
  fills to the tolerance under the lid, and the layers cover each other. Drawers and
  slide-to-click lids only when there's a reason; Shawn's Anachrony drawers are the reference
  for when they're built.
- **Exact measurements, with photos.** Every component is measured. Its photos become a
  textured placeholder in the viewer (next up, M4).
- Fusion designs come in as **STEP** (SolidCraft can't read `.f3d`); a STEP import is the
  likely route to the exact-outline component type.

This replaces the earlier plan to size walls and floors in whole extrusion widths: the style
guide's mm values are used as given.

## v1 slice — decisions (2026-10-27)

- **Spec format is TOML** (`games/<slug>/spec.toml`): readable diffs in git, hand-editable. The
  server notices hand edits within 2 s.
- **`lid_depth` means headroom inside the closed lid above the base rim.** Room for trays =
  inner height + lid depth (+ any lid lift used) − top stack. 0 is right for a telescoping lid that rests on the rim.
- **Cards lie flat; pockets are cut from the top at their own depth** (to the floor when the step
  would be under 3 mm). A shallow pocket in a tall tray sits on solid plastic. That costs filament
  but keeps tokens reachable. Hollowing the underside is later work.
- **Trays stretch to fill their shelf** (up to the bed). Room ≥ 12 mm becomes a "Spare" pocket;
  less goes into the wall. Trays under another layer are padded to the layer height.
- **The packer searches stack depth:** it takes the deepest pockets that let every layer fit
  under the lid, so Claude never picks a depth unless it asks for one (`max_depth` tweak).
- **Tray ids are stable per strategy** (`t-<group>`, `t-cards` …), so tweaks survive spec edits.
  fewest-trays numbers its trays after packing, so `max_depth` tweaks don't apply there.
- **Geometry runs one `solidcraft-cli` per tray in parallel**, cached by tray content. Export
  does one full run, because it also writes the 3MF.
- **The CLI is the driver from a Claude session** (`bid`). The MCP server is a mode of the same
  binary, so it can't drift from the CLI.

## Printed boxes — decisions (2026-10-27)

- **A box can be printed instead of bought** (`printed` on a `box` component; `add_box printed=true`,
  or "Print one — sized to fit" in the box form). Boxes are listed apart, at the top of the
  Components tab.
- **The packer sizes it:** it packs loose at floors from the bed's down in 5 mm steps (both sides
  capped, which pushes it to compact shapes) and at every pocket depth, keeps the smallest box by
  outside volume, then packs again in that box so the trays fill it. `max_inner` caps any side
  (a shelf, another box). Trays sit print-to-print (0.1) inside it, since the box is a print too.
- **It still holds trays**; pockets aren't cut into the box itself. That costs a second set of
  walls, but keeps one packer and lets trays come out. Worth revisiting for one-tray boxes.
- **Slide-on lid, not telescoping:** 45° dovetail grooves under the rim on three sides, open at
  the far end (x = inner length). The lid prints top face down, so the grooves, its chamfered
  edges and the three grip lines at its pull end all need no supports. **It clicks shut** with a
  ridge under the lid's end that drops into a notch across the end wall (the ridge is on the lid
  so it only meets the box at the end of its travel). Walls, floor, groove, slide gap and detent
  are the style's `printed_box` section, all proposed until test-printed.
- The overhang check allows 1e-4 on the normal: an exact 45° face's f32 normal can be 1e-5 past.

## Labels — decisions (2026-10-29)

From Shawn's Anachrony project (`games/anachrony/pieces/`, 25 label parts measured): every label
is **0.6 mm deep, flush with the face it's on**, ~8 mm capitals, and the tray is **recessed under
each letter**, a true inlay. Faces used: the tray's top and pocket floors. So both options are the
same cut, and differ only in whether it is filled:

- **Inlay** (multi-object colouring): the cut is filled by a separate part. In the Bambu project
  the tray is one object of two parts, the tray and `Label`, the label on the game's label slot
  (`set_labels filament=N`; per piece, `set_plate parts:='{"Label": N}'`), as in Shawn's objects.
- **Cut**: the cut left open, one part, one filament.
- **Which one (Shawn, 2026-10-29):** inlay whenever the label prints in a horizontal layer, which
  every top and pocket-floor label does (trays print floor down; a flip keeps them flat). Cut
  alone is for a stencil printed vertically, on a wall. So `labels.mode` stays inlay, and
  `label_mode=cut` on a tray is an override for trying it.
- **Per-pocket text:** a component's `label` (`update_component label=-5`) is what its pocket's
  floor says, when not its name.
- **The spec asks; the packer places.** Tray tweaks `label` (text on the top), `pocket_labels`
  (each pocket's floor names what it holds; a frame's holders take it from the frame) and
  `label_mode`. `labels.rs` finds the largest clear rectangle on the face (clear of edges,
  pockets, open ends, grab holes, finger holes, floor windows and scoop fillets by
  `labels.margin`), reads along or across it, and sizes the capitals up to `labels.height`; with
  room for less than `labels.min_height` the label is left off with a `label_no_room` warning,
  never squeezed. Every label carries its `why`. Labels never change a tray's size.
- **Text widths are measured, not guessed:** the advance of every ASCII character in SolidCraft's
  built-in Liberation Sans, read from its own `sketch.text` output (no kerning, so a line's width
  is the sum). They run a little wide, the safe side.
- **Geometry from existing commands** (no patch): `sketch.create` / `sketch.text` /
  `solid.extrude cut` into the tray. The fill is its own small run (`extrude new`, saved with
  `file.save_mesh` of everything): SolidCraft makes every glyph a body of its own, named by
  itself, so only a document that holds nothing else can save them all.
- **Cost:** each boolean on a body that already has text in it is slow (glyphs are many faces):
  four labels on the Lorcana tokens tray took 11.6 s cut after the scoop fillets, 3.8 s cut before
  them (as built), 0.2 s with none. One sketch per face height didn't help. Trays are cached, so
  only a changed tray pays it.
- **Styles:** `[labels]` depth 0.6 and height 8 are Shawn's (measured from Anachrony); min_height
  4 and margin 1 are proposed.

## Open questions
- `notes` and `goes_with` are recorded but the packer doesn't read them yet: Claude turns them
  into groups, tweaks or storage choices. Candidates for the packer: keep `goes_with` in one
  tray, honour "stacks of N" as a per-pocket item cap.
- Should the top stack be derived from the as-is components instead of entered by hand?
- **glTF export is missing in SolidCraft v0.24.1** (glTF is import only: `file.insert_mesh`,
  `doc.open`). Workaround: `file.save_mesh` per body to binary STL, then our server writes the GLB
  (flat normals, one material per body). Candidate for an upstream PR (`file.export` glTF/GLB
  with body names and appearances). Not a fork patch.
- **No slicer on the server**, so exported STLs are checked as closed, origin-placed and bed-sized,
  but not sliced. Check the first real export in Bambu Studio.
- Whether SolidCraft's 3MF export keeps per-body colour (matters for multicolour labels)
- Whether appearances can be translucent (only matters if we view models in SolidCraft itself)
- Headless rebuild speed for a full box (does the try-a-change-and-see loop stay fast?)
- Whether SolidCraft's web app runs on the iPad with WebGPU (only for the hand-edit escape hatch)
- Watch upstream printing work (Send to Slicer, EasyPrint/Printables) so we don't duplicate it
