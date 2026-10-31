# BoxInsertDesigner

Design 3D-printed board game inserts with an AI agent, from your phone or a browser.
Built on [SolidCraft](https://github.com/tangency-labs/solidcraft).

You measure and photograph the game's components. The agent proposes layouts, and a deterministic
packer checks that each one fits, prints without supports and follows your style guide. You
compare the variations, refine trays by leaving notes on them for the agent, and see the finished
insert in 3D, down to the cards and tokens sitting in their pockets. When you want to shape
something by hand, open the tray in [SolidCraft](https://github.com/tangency-labs/solidcraft): the
app sees your edit, and you can hand it back to the agent, keep it as the tray, or design a new
piece from scratch. Out come print-ready Bambu Studio projects (plates, filament slots and labels)
and STLs.

## How it works

- **Measure and photograph.** The Components tab takes each component's sizes and photos. Mark a
  photo's four corners and the face is straightened into a texture for the 3D view.
- **Layouts.** The agent decides intent (what goes together, what lifts out); the packer does the
  maths: pockets, trays, layers, walls, grab holes and the bed size. Anything that can't work comes
  back as a structured reason, not a broken model.
- **Refine.** Leave "Changes for Claude" on any tray. An agent working the queue, either the app's
  own (with a Claude API key in Settings) or Claude Code driving `bid`, makes the change and
  replies on the tray, which keeps the whole history.
- **Review.** A three.js viewer with layers, a cut view, a motion video of the insert being
  unpacked, and the print plates as they'll go to the printer.
- **Edit in SolidCraft.** Open a tray in SolidCraft's web app, served by this one, and the edit
  comes back to be compared with the generated tray.
- **Print.** Export a Bambu Studio project 3MF (X1C, 0.4 mm, 0.20 mm Standard), a 3MF per box and
  one STL per tray. Labels can be inlaid in their own filament.

The spec (`games/<game>/spec.toml`) is the master copy. Everything else is generated from it, and
your games folder is its own git repository, saved by the server as you work.

## Requirements

- macOS or Windows (x64). Linux isn't tried yet. Windows setup and what's macOS-only:
  [docs/WINDOWS.md](docs/WINDOWS.md)
- Rust (via [rustup](https://rustup.rs)); on macOS also the GitHub CLI (`gh`) to fetch SolidCraft
- An agent: [Claude Code](https://claude.com/claude-code) driving `bid` (or its MCP server), or
  a Claude API key for the app's own agent
- To reach it from a phone: a private network you control, such as Tailscale. Never expose it
  on the open internet: it lets an agent change files.

## Getting started

```bash
tools/fetch-solidcraft.sh          # the pinned SolidCraft CLI and web app, checksummed
                                   # (Windows: tools\fetch-solidcraft.ps1)
cargo build --release
mkdir -p games && cp bid.example.toml games/bid.toml    # address, allowed host names, paths
cp -R examples/ccg-example games/  # optional: a finished insert to look around
target/release/bid-server          # then open http://127.0.0.1:8102/
```

`games/` is ignored by this repo. Make it a git repository of its own (`git init games`) and the
server versions your work: History in the app, `bid history`, named versions and restores.

Drive it from a shell or an agent with `bid`:

```bash
bid help                       # every command, with argument shapes
bid add_component id=deck name="Main deck" kind=cards size=66x91 count=120 stack=58
bid add_photo component=deck file=deck.jpg
bid generate_layouts           # compare strategies for the current game
bid choose_layout strategy=by-group
bid tweak_tray tray=t-tokens max_depth=15
bid export                     # 3MFs and STLs into games/<game>/out/
```

Every change shows up in the open browser tab within a couple of seconds. For Claude Code,
register the MCP server for your user:
`claude mcp add box-insert-designer -- /path/to/BoxInsertDesigner/target/release/bid mcp`.
`tools/requests/AGENT.md` is the brief for an agent working the change-request queue.

To run it as a service on macOS, `launchd/` has LaunchAgent templates for the server and for the daily
SolidCraft upkeep job.

## Example

[`examples/ccg-example`](examples/ccg-example) is a finished insert for a card game box set:
decks in lift-out holders, villain and rules cards, health dials, tokens and a folded board, with
the change notes and agent replies that shaped it and a hand edit made in SolidCraft. Published
art in its photos is pixelated.

A printed insert designed with it: [Lorcana Palace Heist on MakerWorld](https://makerworld.com/en/models/3524093-lorcana-palace-heist-insert#profileId-4028643).

## Tracking SolidCraft

SolidCraft releases almost daily. This repo carries no SolidCraft code: it pins a release tag
(`upstream.toml`), fetches it, and checks the commands it relies on against a recorded contract.
The upkeep job validates every new release and moves the pin when it's green. Whether a
purpose-built tool can keep up with an upstream that moves this fast is an experiment in its own
right; [docs/UPKEEP.md](docs/UPKEEP.md) and [docs/PATCHES.md](docs/PATCHES.md) are the record.

- Plan and architecture: [docs/PLAN.md](docs/PLAN.md)
- What we know about SolidCraft: [docs/RESEARCH.md](docs/RESEARCH.md)

## SolidCraft

BoxInsertDesigner is built on [SolidCraft](https://github.com/tangency-labs/solidcraft), which
does all of its geometry: headless from its CLI for every tray, and in its web app for hand edits.
It is an independent project, not affiliated with or endorsed by Tangency Labs.

## Licence

MIT or Apache 2.0, at your option ([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE)).
`crates/server/assets/bambu/` holds Bambu Studio's printer and filament preset values; see the
note there.
