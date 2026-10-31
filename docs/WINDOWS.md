# Windows

First run on Windows, 2026-10-30: Windows 11 Pro (10.0.26200), x64, a fresh clone of the
`windows` branch. Everything in the core loop works. One fix was needed (below).

## Setup that worked

1. **Toolchain.** The machine had git and nothing else, so both of these came from winget:

   ```powershell
   winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override "--quiet --wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
   winget install --id Rustlang.Rustup -e
   rustup default stable          # stable-x86_64-pc-windows-msvc, rustc 1.99.1
   ```

   Open a new shell afterwards, or add `%USERPROFILE%\.cargo\bin` to `PATH` for the current one.
   The `ring` crate built with the Build Tools' C compiler, and there was nothing else to set.

2. **SolidCraft.** `powershell -ExecutionPolicy Bypass -File tools\fetch-solidcraft.ps1`
   downloaded the portable package and the web app, checked both against the pins and
   `SHA256SUMS.txt`, and printed `solidcraft-cli 0.27.1`.

3. **Build.** `cargo build --release` took about 2 min from cold. `cargo test --release` passed
   41/41.

4. **Games folder.**

   ```powershell
   mkdir games
   copy bid.example.toml games\bid.toml
   xcopy /e /i examples\ccg-example games\ccg-example
   netstat -ano | findstr :8102     # only LISTENING on 127.0.0.1:8102 means it's taken
   ```

   `netstat` lists outbound connections to *other* machines' port 8102 as well (here, the Mac's
   service over Tailscale). Those don't block the local port, so 8102 was free.

5. **Run.** Start `target\release\bid-server.exe` from the checkout root, since `web\`, `styles\`,
   `upstream\` and `upstream.toml` are found relative to the current folder. Then run
   `target\release\bid.exe`.

## What works

- `bid list_games`, `use_game game=ccg-example`, `generate_layouts` (3 variants, by-group fits),
  `choose_layout strategy=by-group`, `export`.
- `export` wrote all 11 STLs (trays plus the three label parts), `ccg-example-insert.3mf`,
  `ccg-example-plates.3mf` and `manifest.json` in about 30 s. The server log had no errors.
  solidcraft-cli's slowest run per build was about 21 s (8 trays, 217 commands, from cold).
- `bid` finds the server from `games\bid.toml` with no `BID_URL` set, and names the machine from
  `COMPUTERNAME`.
- Web app at `http://127.0.0.1:8102/?game=ccg-example`: the 3D insert view (box art, contents,
  trays with their stencilled labels), the Plates tab (3 plates, all fit the X1C bed) and the
  tray view.
- **Edit in SolidCraft**: the SolidCraft web app (v0.27.1) starts under `/solidcraft/` and builds a
  tray from the server's `edit_script` (Decks: 10 steps, none failed). This was checked by
  running `solidcraft.js`'s steps against the app in an iframe. The in-app browser it was tested
  in opens `window.open` in the same tab, which drops the opener that drives the editor. Click it
  once more in a real browser (Edge or Chrome) to see the full new-tab flow.
- `bid add_photo component=decks file=<phone photo>`: a Galaxy S20 photo stored 4032×3024 with
  EXIF orientation 6 came out upright at 1920×2560, with its EXIF removed.
- Not a git repository, so `games\` runs with save versioning off, as on macOS.

## Fixed on this branch

- **`\\?\` paths.** On Windows, `fs::canonicalize` returns verbatim paths
  (`\\?\C:\…\games`). They showed up in the server log and in `export`'s `dir`, and some
  programs don't accept them. `canonical()` in `crates/server/src/main.rs` strips the prefix
  from drive paths on Windows (`cfg(windows)`). Paths on macOS and UNC paths are unchanged.

## Doesn't apply, or isn't done yet

- **Running it as a service.** `launchd/` and the redeploy in `tools/upkeep/upkeep.sh` are macOS
  only, and nothing equivalent exists for Windows (Task Scheduler, or a service wrapper). Here
  it runs by hand.
- **Daily upkeep** (`tools/upkeep/upkeep.sh`) is bash plus launchd and wasn't tried. It already
  moves the Windows pin along with the macOS one.
- **Rebuilding while the server runs.** Windows locks a running `.exe`, so `cargo build` fails
  to replace `bid-server.exe` until the server is stopped. On macOS you build first and restart
  after. On Windows: stop it, build, start it.
- `tools/contract/check.py` and `tools/upstream-probe/` weren't run.
