# SolidCraft upkeep log

`tools/upkeep/upkeep.sh` runs daily at 06:10 (a LaunchAgent; template in `launchd/`).
When there's a new SolidCraft release tag (never `main`, never nightlies), it validates the
release in a worktree: command contract, `cargo build` + `cargo test`, and a smoke build of
every game through both geometry paths with each STL checked.

- **green:** the pin moves automatically.
- **broken:** a Claude session (`tools/upkeep/SESSION.md`) fixes it and lands it, or marks it
  `blocked` for the owner.

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
