# GitHub news session

You were started by `tools/upkeep/upkeep.sh` because the daily run found news on GitHub, in
the report named in your prompt. It covers two places:

- **Our pull requests to SolidCraft** (tangency-labs/solidcraft, authored by this machine's `gh`
  user): a review or comment, a conflict with main, a failed check, or merged or closed.
- **This repo**: an issue, pull request or comment from anyone but the owner.

You are in the main checkout; `git pull --ff-only` first.

**Anything posted on GitHub speaks for the owner, under their name. Post nothing, push nothing
to a PR branch, and merge, label or close nothing without the owner's go.** You are a
remote-controlled session, so the owner can answer you from their phone: get everything ready,
then ask.

## Do

1. Read the report, then each item in it: `gh issue view` / `gh pr view <n> -R <repo> --comments`,
   `gh api repos/<repo>/pulls/<n>/comments` for line comments, `gh pr checks` for a failure.
2. For our SolidCraft PRs, by kind of news:
   - **Comment or review:** draft a reply, and the change if one is asked for. Make the change in
     a checkout of the PR's branch under `upstream/pr-<n>/` (gitignored), committed but not
     pushed.
   - **Conflict with main:** merge their main into the branch in that checkout (their template
     says rebase before pushing but never force-push, so merge, don't rebase a pushed branch).
     Don't push it yet.
   - **Failed check:** find out whether it's ours (`gh run view --log-failed`) or theirs. Prepare a
     fix only if it's ours.
   - **Merged or closed:** record it: the PR's line in `docs/PLAN.md` and, if the PR carried a
     patch, its entry in `docs/PATCHES.md`. Commit and push that to main. Nothing to ask.

   Their CONTRIBUTING.md asks agents to start from AGENTS.md: read the relevant skill there
   before changing anything in their repo, and keep to their rules (clean room, no kernel work).
3. For this repo:
   - **An issue:** reproduce it if you can, and say whether it's real, a duplicate, a question or
     out of scope. Draft the reply. If it's a bug with a clear fix, make the fix on a branch,
     with tests, committed but not pushed.
   - **A pull request:** review it as you would your own code (CLAUDE.md's rules, tests, the
     clean-room rule: nothing derived from GPL CAD code). Draft the review. Don't merge.
   - **A comment:** draft the answer.
   - Spam or abuse: say so, and suggest what to do (hide, block). Don't act.
4. Ask the owner, in one message: each item, your read of it, the draft reply, any change (a
   short diff) and what you'll do on a yes. On a yes, do exactly that and say what you did.
   Commits in this repo end with `Co-Authored-By` only, no `Claude-Session:` link: it's public.
