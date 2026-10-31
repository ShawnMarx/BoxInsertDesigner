# Upstream PR session

You were started by `tools/upkeep/upkeep.sh` because one of our pull requests to SolidCraft
(tangency-labs/solidcraft, authored by this machine's `gh` user) has news: a review or comment,
a conflict with main, a failed check, or it was merged or closed. The news is in the report named
in your prompt. You are in the main checkout; `git pull --ff-only` first.

**These pull requests speak for the owner, under their name. Post nothing, push nothing to a PR
branch, and close nothing without the owner's go.** You are a remote-controlled session, so the
owner can answer you from their phone: get everything ready, then ask.

## Do

1. Read the report, then each PR in it: `gh pr view <n> -R tangency-labs/solidcraft --comments`,
   `gh api repos/tangency-labs/solidcraft/pulls/<n>/comments` for line comments, and
   `gh pr checks <n> -R tangency-labs/solidcraft` for a failure.
2. By kind of news:
   - **Comment or review:** draft a reply, and the change if one is asked for. Make the change in
     a checkout of the PR's branch under `upstream/pr-<n>/` (gitignored), committed but not
     pushed.
   - **Conflict with main:** merge their main into the branch in that checkout (their template
     says rebase before pushing but never force-push, so merge, don't rebase a pushed branch).
     Don't push it yet.
   - **Failed check:** find out whether it's ours (`gh run view --log-failed`) or theirs. Prepare a
     fix only if it's ours.
   - **Merged or closed:** record it: the PR's line in `docs/PLAN.md` and, if the PR carried a
     patch, its entry in `docs/PATCHES.md`. Commit and push that to main (docs only; end the
     message with `Co-Authored-By` only, no `Claude-Session:` link: this repo is public). Then
     you're done.
3. Ask the owner, in one message: what the news is, your draft reply and/or the change (a short
   diff), and what you'll do on a yes. On a yes, post or push exactly that, and say what you did.
4. Their CONTRIBUTING.md asks agents to start from AGENTS.md. Read the relevant skill there before
   changing anything in their repo, and keep to their rules (clean room, no kernel work).
