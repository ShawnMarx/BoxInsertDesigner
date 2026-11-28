# SolidCraft upkeep log

`tools/upkeep/upkeep.sh` runs daily at 06:10 (a LaunchAgent; template in `launchd/`).
When there's a new SolidCraft release tag (never `main`, never nightlies), it validates the
release in a worktree: command contract, `cargo build` + `cargo test`, and a smoke build of
every game through both geometry paths with each STL checked.

- **green:** the pin moves automatically.
- **broken:** a Claude session (`tools/upkeep/SESSION.md`) fixes it and lands it, or marks it
  `blocked` for the owner.

Each run also checks GitHub first: our pull requests to SolidCraft, and this repo's issues,
pull requests and comments from others. News goes to `upstream/upkeep/github.md` and starts a
session (`tools/upkeep/GITHUB.md`) that drafts the reply and waits for the owner. Merged PRs to
SolidCraft are recorded in PLAN.md.

Days with no new release only go to `~/Library/Logs/box-insert-designer-upkeep.log`.

This table is the evidence for PLAN.md's M9 question: what does keeping up with a daily upstream
cost? Run time is the script's wall clock; for `fixed by session` rows, the commit says how long
the session took.

| Date | From | To | Outcome | Contract | Smoke | Run time | Notes |
|---|---|---|---|---|---|---|---|
| 2026-10-28 | v0.24.1 | v0.25.1 | green | changed | lorcana-palace-heist 36996 ms, sample-harbour 12344 ms | 149 s | auto-bumped |
| 2026-10-29 | v0.25.1 | v0.26.0 | blocked | unchanged (6/6) | lorcana-palace-heist 4813 ms, sample-harbour 8673 ms | 108 s | Not the release: `layout::tests::grab_holes_come_in_pairs_clear_of_every_pocket` has failed on main since it was added in 45adc49 (fails at a2fd3a7 with the v0.25.1 pin too; `t-p1-3 runs into the grab holes at 71.5`). Packer fix left for Shawn; v0.26.0 itself looks clean. Rerun upkeep once main is green. |
| 2026-10-29 | v0.25.1 | v0.26.0 | fixed by session | unchanged | anachrony 6 ms, lorcana-palace-heist 1050 ms, sample-harbour 1772 ms | 4 s | Not the release: packer test grab_holes_come_in_pairs_clear_of_every_pocket failed since 45adc49 (a spare across the tray ran through a grab hole); fixed on main in 505f8e4, v0.26.0 unchanged |
| 2026-10-30 | v0.26.0 | v0.27.1 | green | changed | anachrony 16 ms, lorcana-palace-heist 49024 ms, sample-harbour 8483 ms | 150 s | auto-bumped |
| 2026-10-31 | v0.27.1 | v0.28.0 | green | changed | anachrony 15 ms, ccg-example 70288 ms, lorcana-palace-heist 70880 ms, sample-harbour 12318 ms | 285 s | auto-bumped |
| 2026-11-02 | v0.28.0 | v0.30.0 | green | changed | anachrony 15 ms, ccg-example 79003 ms, lorcana-palace-heist 81428 ms, sample-harbour 13155 ms | 193 s | auto-bumped |
| 2026-11-04 | v0.30.0 | v0.30.1 | green | changed | anachrony 15 ms, ccg-example 99666 ms, lorcana-palace-heist 92113 ms, sample-harbour 17830 ms | 231 s | auto-bumped |
| 2026-11-12 | v0.30.1 | v0.40.1 | green | changed | anachrony 9 ms, ccg-example 69808 ms, lorcana-palace-heist 64841 ms, sample-harbour 16545 ms | 287 s | auto-bumped |
| 2026-11-13 | v0.40.1 | v0.41.0 | green | changed | anachrony 13 ms, ccg-example 65546 ms, lorcana-palace-heist 65866 ms, sample-harbour 14058 ms | 171 s | auto-bumped |
| 2026-11-15 | v0.41.0 | v0.42.0 | green | unchanged | anachrony 10 ms, ccg-example 71535 ms, lorcana-palace-heist 65745 ms, sample-harbour 14696 ms | 180 s | auto-bumped |
| 2026-11-16 | v0.42.0 | v0.43.0 | green | changed | anachrony 10 ms, ccg-example 64465 ms, lorcana-palace-heist 64070 ms, sample-harbour 14357 ms | 169 s | auto-bumped |
| 2026-11-18 | v0.43.0 | v0.44.1 | green | changed | anachrony 438 ms, ccg-example 79622 ms, lorcana-palace-heist 77161 ms, sample-harbour 21863 ms | 207 s | auto-bumped |
| 2026-11-20 | v0.44.1 | v0.45.0 | green | unchanged | anachrony 11 ms, ccg-example 95601 ms, lorcana-palace-heist 89947 ms, sample-harbour 26301 ms | 245 s | auto-bumped |
| 2026-11-23 | v0.45.0 | v0.47.0 | green | changed | anachrony 9 ms, ccg-example 75802 ms, lorcana-palace-heist 69737 ms, sample-harbour 23972 ms | 203 s | auto-bumped |
| 2026-11-24 | v0.47.0 | v0.48.1 | green | changed | anachrony 16 ms, ccg-example 78450 ms, lorcana-palace-heist 69750 ms, sample-harbour 23279 ms | 207 s | auto-bumped |
| 2026-11-25 | v0.48.1 | v0.49.0 | green | changed | anachrony 17 ms, ccg-example 72349 ms, lorcana-palace-heist 72767 ms, sample-harbour 28237 ms | 207 s | auto-bumped |
| 2026-11-26 | v0.49.0 | v0.49.1 | green | changed | anachrony 12 ms, ccg-example 96086 ms, lorcana-palace-heist 95030 ms, sample-harbour 33347 ms | 259 s | auto-bumped |
| 2026-11-27 | v0.49.1 | v0.50.0 | green | changed | anachrony 17 ms, ccg-example 116153 ms, galactic-cruise 2 ms, lorcana-palace-heist 105630 ms, sample-harbour 32191 ms | 435 s | auto-bumped |
| 2026-11-28 | v0.50.0 | v0.51.2 | green | changed | anachrony 15 ms, ccg-example 71315 ms, galactic-cruise 2 ms, lorcana-palace-heist 60105 ms, sample-harbour 19165 ms | 183 s | auto-bumped |
