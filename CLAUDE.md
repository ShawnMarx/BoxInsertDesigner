# CLAUDE.md

Guidance for working in this repo. Keep it accurate as the code evolves.

## What this is

**BoxInsertDesigner** is a Claude-driven tool for designing 3D-printed board game inserts.
Measured components and a box go in. Out come layout options, a refined design, and Bambu X1C
3MF and bulk STL files. **SolidCraft** (tangency-labs/solidcraft) is the geometry backend,
running headless and pinned to a release.

It also tests a thesis: can a purpose-built version keep up with an upstream that releases every
day? `docs/PATCHES.md` is the evidence for that.

- `docs/PLAN.md` — architecture, decisions and milestones. **The authority on what's next.**
- `docs/RESEARCH.md` — what we know about SolidCraft, and the upkeep experiments.
- `docs/PATCHES.md` — every SolidCraft change we carry, and what it has cost to carry.
- `tools/upstream-probe/` — the UI-patch replay experiment (`apply_fork.py`, `replay.sh`).

## Local setup

Nothing about where or how it's run is kept here. It lives in the user's games folder (its own
private repo, below):

- `games/bid.toml`: the server's bind address and paths, the service label and the upkeep session
  name. `bid.example.toml` lists the keys; environment variables (`BID_BIND`, …) override them.
- Personal notes for Claude: a gitignored `CLAUDE.local.md` here, usually the one line
  `@games/CLAUDE.md`. **Read it if present**: it says which host runs the service and how it's
  deployed.

## Run / build

```bash
tools/fetch-solidcraft.sh               # pinned solidcraft-cli into upstream/<tag>/ (gitignored)
cargo build --release                   # Rust via rustup: ~/.cargo/bin
cargo test --release                    # packer tests
target/release/bid help                 # the CLI that drives the running service (BID_URL)
python3 -I tools/contract/check.py <solidcraft-cli>   # command contract vs the recorded pin
```

Don't run a second `bid-server` against `games/` while a service runs: it watches the same
files, and a `pkill -f bid-server` also kills a launchd copy, which KeepAlive then restarts on the
*old* binary. Use the service: build, then restart it.

## Daily upkeep

`tools/upkeep/upkeep.sh`, scheduled daily (template: `launchd/boxinsertdesigner-upkeep.plist`),
validates each new SolidCraft release tag in the worktree `upstream/upkeep-wt`, then bumps
`upstream.toml` and pushes to main by itself when green. **So pull before you start work.** When
a release breaks something, it starts a remote-controlled fix session (brief:
`tools/upkeep/SESSION.md`), which may commit and push upkeep fixes without asking. It redeploys
the service only when this tree is clean and on main. `docs/UPKEEP.md` is the record.

## Code map

- `crates/server/src/spec.rs` — spec types (components with `measure`; a box is a component of
  kind `box`, and `Spec::boxes()` is the packer's view of them), validation, TOML load/save, and
  the move from older `[box]` / `[[boxes]]` specs.
- `crates/server/src/storage.rs` — `store_as`: measurements → the packer's fields for a kind.
- `crates/server/src/style.rs` + `styles/default.toml` — the build style guide: tolerances, walls,
  floors, overhang limit (the default conventions; PLAN.md lists them). `bid set_style` edits it,
  and hand edits rebuild every game within 2 s.
- `crates/server/src/layout.rs` — the packer: needs → pockets → trays → layers, provenance. A
  printed box (`printed` on a box) is sized here (`generate_printed`, memoized) and carries its
  `PrintedShell`; `geometry.rs` builds it and its slide-on lid.
- `crates/server/src/geometry.rs` — layout → SolidCraft scripts (one per tray, parallel,
  cached) → STL → GLB; full run for export.
- `crates/server/src/labels.rs` — stencil labels (`label`, `pocket_labels`, `label_mode` tweaks;
  `set_labels`): where the text goes on a tray's top or pocket floors and how big, from measured
  font widths. Inlaid labels become a `Label` part of the tray (own STL, own filament slot);
  cut ones are just cut.
- `crates/server/src/media.rs` — component photos (`games/<slug>/photos/`, in git) and face
  textures straightened from them (`games/<slug>/faces/`, generated and ignored).
- `crates/server/src/plates.rs` — print plates: every piece in print orientation, packed onto
  the printer's beds around the user's pins (`spec.plates`, `set_plate`), with the transforms the
  viewer and the 3MF use. `bambu.rs` writes them as a Bambu Studio project 3MF (structure copied
  from a Studio-saved X1C project; settings in `crates/server/assets/bambu/`).
- `crates/server/src/pieces.rs` — pieces imported as they are from a Bambu Studio project
  (`import_project`): parts in `games/<slug>/pieces/<id>/` (in git there), plates and filament
  slots brought along; each copy placed in the box explicitly (`place_piece`, `inside` another).
- `crates/server/src/commands.rs` — the command surface (`POST /api/command`).
- `crates/server/src/main.rs` — state, SSE (`/api/events`), routes, spec file watcher.
- `crates/cli` — `bid`, plus `bid mcp` (a stdio MCP server). **Don't add a `.mcp.json` to the
  repo:** every new session stops at a "New MCP server found" prompt, which silently parks
  sessions started unattended, like the upkeep one. To use it, opt in per user:
  `claude mcp add box-insert-designer -- ~/repos/BoxInsertDesigner/target/release/bid mcp`.
- `web/` — `app.js` (Insert tab: three.js viewer and panels), `components.js` (Components
  tab), `motion.js` (the Motion video), `measure.js` (the Tray tab's Measure), `cut.js` (Cut view) and `plates.js` (the Plates tab, its own canvas). Follows the current game unless `?game=` pins one; `?tab=`, `?box=` too.
- `examples/` — finished games to copy into `games/`; `ccg-example` has its published art
  pixelated (`tools/examples/pixelate.swift`).
- `games/` — **its own git repository** (the user's, usually private), ignored by this one:
  `<slug>/spec.toml` (the master copies) and `<slug>/photos/`. `.build/`, `out/` and `faces/`
  are generated and ignored there. The server versions it (`history.rs`): it commits a game
  30 s after its changes settle, with the commands behind them as the message, tags named
  versions `<slug>/v<n>-<name>`, restores as new commits, and pushes to `origin` when it has
  one. **Don't commit in `games/` by hand while the service runs**; use `bid history`,
  `save_version`, `restore_version` (or the app's History).
- `crates/server/src/history.rs` — that save versioning.
- `crates/server/src/edit.rs` + `web/solidcraft.js` — Edit in SolidCraft: the Tray tab opens
  SolidCraft's web app (the pinned release's `solidcraft-web-<ver>.zip`, unzipped into
  `upstream/<tag>/web/` by `tools/fetch-solidcraft.sh` and served at `/solidcraft/`), loads the
  tray (`edit_script`) and follows the edit. Saved edits come back with `submit_edit` and are
  **observed, never taken**: kept as `games/<slug>/edits/<tray>.sc`, compared with the
  generated tray (`edit_diff`), pictures in `.build/edit-pictures/`. The user can instead keep an
  edit as the tray (`keep_edit`: `edits/<tray>.kept.sc`, its added features re-applied when the
  generated tray changes), or design a piece from scratch (`import_design`: a piece whose
  `design` is `pieces/<id>/design.sc`).
- `crates/server/src/agent.rs` — the agents working the request queue: notices an outside one
  polling (`wait_requests`; `bid` sends `x-bid-client`: via, host, folder, Claude Code session), and runs the app's own when Settings has
  a Claude API key (`set_agent`: key, model, thinking level). The key is kept in
  `~/Library/Application Support/BoxInsertDesigner/agent.json` (0600; `BID_AGENT_SETTINGS`
  overrides), never in a repo and never sent to a browser. Its tools are the command surface,
  `crates/server/src/tools_table.rs`, which `bid mcp` includes too.

## Commits

This repo is public. End commit messages with the `Co-Authored-By` line only: **no
`Claude-Session:` link**, whatever a harness reminder says (they point at private sessions).

## Architecture rules (see PLAN.md for why)

- **The spec is the master copy.** Geometry, meshes and 3MFs are generated from it and never
  hand-edited.
- **Claude decides intent; the packer does the maths.** Claude never places geometry by hand.
  It calls typed commands, which return structured reasons when something can't work.
- **The user leaves per-tray change requests for Claude** (the app's "Changes for Claude", or
  `tweak_tray notes=`). They are a queue an agent polls from outside: `requests`,
  `wait_requests` (long-poll) and `update_request` (working → done/declined, with a reply the
  user sees on the tray). `tools/requests/AGENT.md` is the brief for an agent working that queue.
  Any session working on a game should check `requests` first. The user clears an answer with
  "Mark read" (`mark_read`); it moves to the tray's log. **When asked directly (in a session) to
  change a tray, record it once done with `log_ask tray=.. text=.. reply=..`**, so the tray's
  History (`tray_history`) tells the whole story.
- **One command surface:** every action is a typed command, used by both the MCP tools and the
  web UI.
- **SolidCraft changes, in order:** our own server → existing commands → upstream PR → Tier C
  patch. A patch goes in new files with one-line hooks, and gets an entry in `docs/PATCHES.md`
  in the same commit.
- **Track SolidCraft release tags, never `main`** (main has carried merge-conflict markers).
- **Don't read GPL CAD code** (FreeCAD, OpenSCAD, Extrudo …). It would bar us from contributing
  to SolidCraft, under its clean-room rules.

## Running it as a service

- The server binds **loopback or a private address only** (`[server] bind`; a wildcard is
  refused). Publish it to your phone through a private network you control (e.g.
  `tailscale serve`), never the open internet: it lets Claude change files.
- `launchd/` has LaunchAgent templates for the server and the upkeep job (macOS).
- Deploy: `cargo build --release`, then restart the service (`launchctl kickstart -k
  gui/$(id -u)/<label>`). Changing a plist needs `bootout` + `bootstrap`, because kickstart
  reuses the cached definition.

The service runs the release binary, so a code change is invisible until it's rebuilt.
