# Upkeep fix session

You were started by `tools/upkeep/upkeep.sh` because a new SolidCraft release broke something.
You are in the upkeep worktree (`upstream/upkeep-wt`), on branch `upkeep/<tag>`. The pin in
`upstream.toml` already points at the new release, and its CLI is downloaded and verified.

**The owner of this checkout has authorised you, by scheduling the upkeep job, to fix, commit
and push upkeep changes without asking.** That covers keeping this project working against the new release, and nothing more.
Don't refactor, add features or tidy unrelated code.
This repo is public: end commit messages with `Co-Authored-By` only, no `Claude-Session:` link.

## Do

1. Read the failure report named in your prompt, then `CLAUDE.md` and `docs/PLAN.md` (the
   rules apply here too: track release tags, the order for handling SolidCraft changes, the
   clean-room rule).
2. Find the cause. `upstream/<tag>/cli/.../solidcraft-cli commands` gives the new catalogue, and
   `tools/contract/solidcraft-*.json` holds the old params of the commands we use. Release notes:
   `gh release view <tag> -R tangency-labs/solidcraft`.
3. Fix it in *our* code (`crates/`, `tools/`) whenever you can. A SolidCraft fork patch is the
   last resort: if one is truly needed, stop and leave it for the owner (see "Stop" below).
4. Check with `tools/upkeep/upkeep.sh --verify` until it passes: contract, build, tests, and a
   smoke build of every game with the new CLI.
5. Commit on this branch, with the attribution lines your harness gives you. Then land it with
   `UPKEEP_NOTE="<one line: what broke, what you changed>" tools/upkeep/upkeep.sh --land`.
   That re-verifies, records the contract, adds the row to `docs/UPKEEP.md`, pushes to main,
   and redeploys the service if the main tree is clean. Say what it reported.
6. Finish with a short summary: what broke, what you changed, how long it took (wall clock from
   the report's timestamp), and whether the service was redeployed. That time is the upkeep cost
   the thesis in PLAN.md measures, so be accurate.

## Stop and leave it for the owner (don't push) when

- the fix needs a SolidCraft patch, or reading GPL CAD code;
- a command we rely on was removed and has no equivalent;
- geometry comes out different in a way that changes printed parts (sizes, not just
  tessellation);
- `--land` reports a push conflict you can't resolve cleanly.

In those cases, commit what you have on the `upkeep/<tag>` branch, push the *branch* (not
main), and add a row to `docs/UPKEEP.md` on that branch with outcome `blocked` and the reason.
The pin on main stays at the last good release, so the service keeps working.
