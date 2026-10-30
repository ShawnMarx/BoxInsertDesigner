# Research — SolidCraft as a backend (2026-10-27)

## The project
- **Repo:** `tangency-labs/solidcraft` (formerly `bherbruck/solidcraft`), created 2026-10-07.
  MIT OR Apache-2.0. ~11k stars on 2026-10-27.
- **Language and pace:** pure Rust. Releases every day (v0.24.1 on 2026-10-27), with nightly
  builds in between. Mostly agent-written; its `AGENTS.md` and skills define the rules.
- **Layers:** `geom` → `sketch`/`kernel`/`render` → `doc` → `io` → `engine` (command registry) →
  `gpu`/`ui-egui`/`mcp` → apps (`solidcraft`, `solidcraft-cli`).
- **How it can be driven:** everything is a command. The MCP server (`solidcraft-cli mcp`) runs
  headless or bridged to the app with `--connect`. Scripts are JSON lists of commands. The
  browser build exposes `window.solidcraft`, in-page only, with no network listener.
- **Agent chat:** a desktop-only panel that runs Claude Code and hands it the app's own MCP
  server. "Attach selection" sends what you've picked.
- **Plugins:** designed (`docs/plugins.md`, WebAssembly components with a WIT interface,
  parametric `FeatureKind::Plugin`). Issue #1221 is still open, and nothing runs plugins yet.
- **Kernel:** being migrated from truck to SMLib on the `kernel-smlib` branch (#4012). Expect
  geometry behaviour to change when it lands.
- **Printing upstream:** Send to Slicer (#3600, merged), EasyPrint upload to Printables
  (v0.24.1), 3MF import with Bambu object names.
- **Clean-room rules for upstream PRs:** no GPL code or reading GPL code (FreeCAD, OpenSCAD,
  OpenCASCADE …). This applies to anything we want upstream.
- **File format:** designs only open forward (a newer save won't open in older versions). The
  spec, not the design file, is our master copy.

## Experiment 1 — replaying a UI patch (tools/upstream-probe/)
`apply_fork.py` makes four edits in `ui-egui`: a `GAME BOX` tab, its toolbar section, a sorted
`pub mod` line, and a viewport overlay hook. `replay.sh` merges each release from v0.18.0 to
v0.24.1 into that fork, and re-applies the edits when a merge conflicts.

| | Result |
|---|---|
| Upstream commits in range | 806 (7 days) |
| Lines changed in the 3 touched files | +5,414 / −732 |
| Clean merges | 12 of 13 releases |
| Conflicts | 1, at v0.24.1, in `lib.rs`: upstream added `pub mod insert_handles;` next to ours |
| Re-applying the edits from scratch | all 4 edits found their place on every release |

**Lessons:**
- Re-applying the fork's changes beats `git merge`.
- Keep fork code in new files, with one-line hooks.
- Hot spots: `ui-egui/src/lib.rs` (116 commits/week), `toolbar.rs` (57) and `prefs.rs` (62).
  On the backend: `engine/src/lib.rs` (50) and `cmd/features.rs` (36).

**Not tested:** compiling. A clean text merge can still break the build.

## Experiment 2 — command contract (v0.18.0 → v0.24.1)
Commands went from 404 to 482, with **none removed or renamed**. All 19 commands this tool
would use are present in both versions. Parameter schemas were not compared; that's M1.

## Experiment 3 — headless geometry, v0.24.1 (2026-10-27)
How fast can SolidCraft rebuild a whole insert headless? (This was an open question in PLAN.md.)

- **Script format:** `{"commands": [{"command": "solid.box", "params": {...}}, ...]}` and
  `solidcraft-cli run script.json`. Process start plus a trivial script takes ~40 ms.
- **Commands used:** `solid.box` / `solid.cylinder` with `operation: new|cut` and `targets`,
  `solid.fillet` (edges picked by a point on each edge, `fit: reduce`), `solid.move`,
  `file.save_mesh` (per body, STL or 3MF), `inspect.measure`. Pass `operation: "new"` explicitly
  for a body that overlaps another, or the default may join them.
- **Imports (v0.24.1):** exact B-rep from STEP, IGES, FreeCAD `.FCStd` and Rhino `.3dm`
  (`file.insert_step`); meshes from 3MF, STL, OBJ and glTF (`file.insert_mesh`). **No Fusion
  360 `.f3d`/`.f3z`:** export STEP from Fusion. That's the likely route for exact-outline
  components and for existing designs (e.g. drawer-style inserts).
- **No glTF export** in v0.24.1, only import. `file.save_mesh` to 3MF keeps body names as object
  names.
- **Errors:** a refused *command* (e.g. a fillet point that misses an edge) aborts the whole run
  even with `--allow-errors`. That flag only lets *timeline features* that fail through. So the
  server retries a tray without scoops if a fillet is refused.
- **Timings** (an M-series Mac mini; `--timings`):

| Build | Commands | Time |
|---|---|---|
| Sample box, one script, 5 trays, 9 per-pocket fillets | 135 | 2.45 s (fillets 1.63 s, boxes 0.36 s, cylinders 0.27 s, STL saves 0.20 s) |
| Same, one run per tray in parallel | ~180 total | 1.5–3.5 s (the slowest tray) |
| + fillets batched per radius, after all cuts | ~180 total | **0.55 s** (by-kind; was 3.5 s) |
| Rebuild where every tray is cached | ghosts only | **0.15–0.2 s** |

  **Fillets compound:** each fillet on a body made the next one slower (195 → 244 → 291 →
  346 ms), and it slowed every later cut on that body too. Do cuts first and fillets last, in
  one command per radius.
- **End to end** (`bid` command → change visible in a browser tab over the private network, no reload):
  the panels update in 0.2–0.8 s; new geometry appears in about 1–2 s. Before the parallel and
  batched builds it took 5–6 s.

## Alternatives looked at
- **Extrudo** (zoltanf/extrudo): browser CAD for 3D printing. GPL-3.0, one main author. Not
  usable as a base, and reading its code would bar us from contributing to SolidCraft.
