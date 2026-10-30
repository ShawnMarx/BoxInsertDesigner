#!/bin/bash
# Daily SolidCraft upkeep (launchd/boxinsertdesigner-upkeep.plist). Tracks release tags,
# never main.
#
#   upkeep.sh            new release? fetch + verify it, then validate it in a worktree
#   upkeep.sh --force    validate the latest release even if it is already pinned (no bump)
#   upkeep.sh --verify   validate the worktree as it stands (the fix session uses this)
#   upkeep.sh --land     verify, then record, push to main and deploy (the fix session ends here)
#   --no-session         on breakage, write the report but don't start the fix session
#
# Validation runs in a git worktree (upstream/upkeep-wt) with its own cargo target, so neither
# your working tree nor the running service binary changes until everything is green:
# contract check, cargo build + test, and `bid-server --smoke` (every game, live and export
# paths, every STL checked) against the new solidcraft-cli.
#
# Green: bump upstream.toml, re-record the contract, add a row to docs/UPKEEP.md, commit, push
# to main, and redeploy the service if the main tree is clean on main.
# Red: write upstream/upkeep/report-<tag>.md and start a remote-controlled Claude session in the
# worktree (tools/upkeep/SESSION.md) that fixes, commits and lands it. Scheduling this job is the
# authorisation: the session may commit and push upkeep fixes without asking.
#
# Local setup comes from bid.toml in the games folder (bid.example.toml): [service] label is the
# LaunchAgent to restart on deploy (none: no restart), [server] bind the address to health-check,
# [upkeep] session the fix session's name.

set -uo pipefail
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin:$HOME/.local/bin"

# The main checkout, also when this copy of the script runs from the worktree.
REPO=$(dirname "$(git -C "$(dirname "$0")" rev-parse --path-format=absolute --git-common-dir)")
conf() { # section.key [default]: from the games folder's bid.toml
  python3 -I -c 'import sys,tomllib
try: d=tomllib.load(open(sys.argv[1],"rb"))
except OSError: d={}
s,k=sys.argv[2].split(".")
print(d.get(s,{}).get(k,sys.argv[3]))' "${BID_GAMES:-$REPO/games}/bid.toml" "$1" "${2:-}"
}
WT="$REPO/upstream/upkeep-wt"
TARGET="$REPO/upstream/upkeep-target"
REPORTS="$REPO/upstream/upkeep"
GH_REPO="tangency-labs/solidcraft"
SESSION=$(conf upkeep.session bid-upkeep)
LABEL=$(conf service.label)
BIND=$(conf server.bind 127.0.0.1:8102)
SCREEN=/opt/homebrew/bin/screen
CLAUDE="$HOME/.local/bin/claude"

mode=run; session=1
for a in "$@"; do
  case "$a" in
    --force) mode=force ;; --verify) mode=verify ;; --land) mode=land ;;
    --no-session) session=0 ;;
    *) echo "unknown option $a" >&2; exit 2 ;;
  esac
done

log() { echo "$(date -Iseconds) $*"; }
toml_get() { sed -n "s/^$1 *= *\"\(.*\)\"/\1/p" "$2"; }
started=$(date +%s)
mkdir -p "$REPORTS"

lock="$REPORTS/.lock"
if ! mkdir "$lock" 2>/dev/null; then
  log "another upkeep run holds $lock; exiting"; exit 0
fi
trap 'rmdir "$lock" 2>/dev/null' EXIT

# ---- the release ----------------------------------------------------------------------------
fetch_release() { # tag → sets ASSET SHA BINARY CLI; downloads into upstream/<tag>/ and verifies
  local tag=$1 dir="$REPO/upstream/$1" ver="${1#v}"
  ASSET="solidcraft-cli-${ver}-macos-universal.zip"
  mkdir -p "$dir"
  local base="https://github.com/$GH_REPO/releases/download/$tag"
  [ -f "$dir/SHA256SUMS.txt" ] || curl -fsSL -o "$dir/SHA256SUMS.txt" "$base/SHA256SUMS.txt" || return 1
  [ -f "$dir/$ASSET" ] || curl -fsSL -o "$dir/$ASSET" "$base/$ASSET" || { log "no $ASSET in $tag"; return 1; }
  SHA=$(awk -v a="$ASSET" '$2 == a { print $1 }' "$dir/SHA256SUMS.txt")
  local actual; actual=$(shasum -a 256 "$dir/$ASSET" | awk '{ print $1 }')
  if [ -z "$SHA" ] || [ "$SHA" != "$actual" ]; then
    log "SHA256 MISMATCH for $ASSET: listed '$SHA', file $actual"; rm -f "$dir/$ASSET"; return 1
  fi
  rm -rf "$dir/cli" && unzip -q "$dir/$ASSET" -d "$dir/cli" || return 1
  BINARY="solidcraft-cli-${ver}-macos-universal/solidcraft-cli"
  CLI="$dir/cli/$BINARY"
  [ -x "$CLI" ] || { log "no $BINARY in $ASSET"; return 1; }
  # The web app (Edit in SolidCraft) is optional: without it the editor says it's missing.
  WEB_ASSET="solidcraft-web-${ver}.zip"; WEB_SHA=""
  if [ -f "$dir/$WEB_ASSET" ] || curl -fsSL -o "$dir/$WEB_ASSET" "$base/$WEB_ASSET"; then
    WEB_SHA=$(awk -v a="$WEB_ASSET" '$2 == a { print $1 }' "$dir/SHA256SUMS.txt")
    if [ -n "$WEB_SHA" ] && [ "$WEB_SHA" = "$(shasum -a 256 "$dir/$WEB_ASSET" | awk '{ print $1 }')" ]; then
      rm -rf "$dir/web" && unzip -q "$dir/$WEB_ASSET" -d "$dir/web" || WEB_SHA=""
    else
      log "SHA256 MISMATCH for $WEB_ASSET; no web app for $tag"; rm -f "$dir/$WEB_ASSET"; WEB_SHA=""
    fi
  else
    log "no $WEB_ASSET in $tag"
  fi
}

set_pin() { # rewrite upstream.toml in the worktree
  /usr/bin/python3 -I - "$WT/upstream.toml" "$1" "$ASSET" "$SHA" "$BINARY" "$WEB_ASSET" "$WEB_SHA" <<'EOF'
import re, sys
path, tag, asset, sha, binary, web_asset, web_sha = sys.argv[1:]
s = open(path).read()
web = (("web_asset", web_asset), ("web_sha256", web_sha)) if web_sha else (("web_asset", ""), ("web_sha256", ""))
for k, v in (("tag", tag), ("asset", asset), ("sha256", sha), ("binary", binary)) + web:
    s = re.sub(rf'^{k} = ".*"$', f'{k} = "{v}"', s, flags=re.M)
open(path, "w").write(s)
EOF
}

# ---- validation (in the worktree) -----------------------------------------------------------
REPORT=""
step() { # name, command...: run, keep the tail of its output for the report
  local name=$1; shift
  local out rc t0=$(date +%s)
  out=$("$@" 2>&1); rc=$?
  STEP_OUT="$out"
  REPORT+=$'\n'"## $name — $([ $rc -eq 0 ] && echo ok || echo "FAILED (exit $rc)") in $(( $(date +%s) - t0 )) s"$'\n```\n'"$(printf '%s\n' "$out" | tail -40)"$'\n```\n'
  log "$name: $([ $rc -eq 0 ] && echo ok || echo "FAILED rc=$rc")"
  return $rc
}

validate() { # uses $CLI; sets CONTRACT SMOKE; returns 1 on breakage
  local ok=0
  CONTRACT="unchanged"
  if ! step "contract" /usr/bin/python3 -I "$WT/tools/contract/check.py" "$CLI"; then
    # Prose changes to a command's params are normal; a MISSING command is a break, and any
    # change is only fine if the build, tests and smoke below still pass.
    if printf '%s' "$STEP_OUT" | grep -q '^MISSING'; then ok=1; CONTRACT="MISSING commands"; else CONTRACT="changed"; fi
  fi
  step "cargo build" env CARGO_TARGET_DIR="$TARGET" cargo build --release --manifest-path "$WT/Cargo.toml" || ok=1
  step "cargo test" env CARGO_TARGET_DIR="$TARGET" cargo test --release --manifest-path "$WT/Cargo.toml" || ok=1
  SMOKE="not run"
  if [ -x "$TARGET/release/bid-server" ]; then
    if step "smoke" bash -c "cd '$WT' && BID_SOLIDCRAFT='$CLI' '$TARGET/release/bid-server' --smoke '$REPO/games'"; then
      SMOKE=$(printf '%s\n' "$STEP_OUT" | /usr/bin/python3 -I -c "
import json, sys
rows = [json.loads(l) for l in sys.stdin if l.startswith('{')]
print(', '.join(f\"{r['game']} {r['ms']} ms\" for r in rows) or 'no games')")
    else
      SMOKE="FAILED"; ok=1
    fi
  fi
  return $ok
}

# ---- landing ----------------------------------------------------------------------------------
land() { # from, to, outcome, notes: record, commit, push, deploy
  local from=$1 to=$2 outcome=$3 notes=$4
  /usr/bin/python3 -I "$WT/tools/contract/check.py" "$CLI" record >/dev/null
  local row="| $(date +%Y-%m-%d) | $from | $to | $outcome | $CONTRACT | $SMOKE | $(( $(date +%s) - started )) s | $notes |"
  printf '%s\n' "$row" >> "$WT/docs/UPKEEP.md"
  git -C "$WT" add -A upstream.toml tools/contract docs/UPKEEP.md
  git -C "$WT" commit -q -m "upkeep: SolidCraft $from -> $to ($outcome)

$row" || { log "nothing to commit"; }
  if ! git -C "$WT" push -q origin HEAD:main; then
    log "push rejected; rebasing onto origin/main"
    git -C "$WT" fetch -q origin && git -C "$WT" rebase -q origin/main && git -C "$WT" push -q origin HEAD:main || { log "PUSH FAILED"; return 1; }
  fi
  log "pushed $(git -C "$WT" rev-parse --short HEAD) to main"
  deploy
}

deploy() { # only when the main tree is clean and on main: never disturb work in progress
  local branch dirty
  branch=$(git -C "$REPO" branch --show-current)
  dirty=$(git -C "$REPO" status --porcelain)
  if [ "$branch" != "main" ] || [ -n "$dirty" ]; then
    log "NOT DEPLOYED: main tree is on '$branch'$([ -n "$dirty" ] && echo ' with uncommitted changes'); pull and rebuild when ready"
    return 0
  fi
  git -C "$REPO" pull -q --ff-only origin main || { log "NOT DEPLOYED: pull --ff-only failed"; return 0; }
  (cd "$REPO" && cargo build --release -q) || { log "DEPLOY FAILED: cargo build in the main tree"; return 1; }
  if [ -z "$LABEL" ]; then log "built; no [service] label in bid.toml, so restart the server yourself"; return 0; fi
  launchctl kickstart -k "gui/$(id -u)/$LABEL"
  sleep 4
  curl -fsS "http://$BIND/health" >/dev/null && log "deployed; service healthy" || log "DEPLOY WARNING: /health not answering"
}

# ---- the fix session --------------------------------------------------------------------------
start_session() { # report path
  local report=$1
  if "$SCREEN" -ls 2>/dev/null | grep -q "\.${SESSION}[[:space:]]"; then
    log "a $SESSION session is already running; leaving it (report: $report)"; return 0
  fi
  # A session in an untrusted directory parks on the trust prompt, detached and invisible.
  if ! /usr/bin/python3 -I -c "import json,os,sys; d=json.load(open(os.path.expanduser('~/.claude.json'))); sys.exit(0 if d.get('projects',{}).get(sys.argv[1],{}).get('hasTrustDialogAccepted') else 1)" "$WT"; then
    log "WARNING: $WT is not trusted in ~/.claude.json; the session will wait on the trust prompt (screen -r $SESSION to answer it)"
  fi
  local prompt="Daily SolidCraft upkeep found a break. Read tools/upkeep/SESSION.md and follow it. The failure report is $report."
  local inner="$SCREEN -dmS $SESSION $CLAUDE '$prompt' --permission-mode auto --remote-control $SESSION"
  local shell_cmd="cd '$WT' && $inner && sleep 1 && exec $SCREEN -r $SESSION"
  # Through Terminal.app, so a TCC prompt can't hang a detached process.
  if ! /usr/bin/osascript -e "tell application \"Terminal\" to do script \"$shell_cmd\"" >/dev/null 2>&1; then
    log "Terminal.app spawn failed; falling back to a bare detached screen"
    (cd "$WT" && "$SCREEN" -dmS "$SESSION" "$CLAUDE" "$prompt" --permission-mode auto --remote-control "$SESSION")
  fi
  sleep 3
  "$SCREEN" -ls 2>/dev/null | grep -q "\.${SESSION}[[:space:]]" && log "started session $SESSION in $WT" || log "FAILED to start $SESSION"
}

# ---- main -------------------------------------------------------------------------------------
pinned=$(toml_get tag "$REPO/upstream.toml")

if [ "$mode" = verify ] || [ "$mode" = land ]; then
  tag=$(toml_get tag "$WT/upstream.toml")
  CLI="$REPO/upstream/$tag/cli/$(toml_get binary "$WT/upstream.toml")"
  log "verifying worktree $(git -C "$WT" rev-parse --short HEAD) against $tag"
  if ! validate; then printf '%s\n' "$REPORT"; log "VERIFY FAILED"; exit 1; fi
  [ "$mode" = verify ] && { printf '%s\n' "$REPORT"; log "verify ok"; exit 0; }
  land "$pinned" "$tag" "fixed by session" "${UPKEEP_NOTE:-see commit}"; exit $?
fi

latest=$(curl -fsSL "https://api.github.com/repos/$GH_REPO/releases/latest" | /usr/bin/python3 -I -c "import json,sys; print(json.load(sys.stdin)['tag_name'])")
[ -n "$latest" ] || { log "could not read the latest release"; exit 1; }
if [ "$latest" = "$pinned" ] && [ "$mode" != force ]; then
  log "no new release (pinned $pinned is latest)"; exit 0
fi
log "=== upkeep: pinned $pinned, latest $latest ==="

git -C "$REPO" fetch -q origin
if [ -d "$WT" ]; then
  git -C "$WT" checkout -q --detach origin/main && git -C "$WT" reset -q --hard origin/main && git -C "$WT" clean -qfd
else
  git -C "$REPO" worktree add -q --detach "$WT" origin/main
fi

if ! fetch_release "$latest"; then
  REPORT+=$'\n## fetch\nCould not download or verify the release CLI.\n'
  outcome=broken
else
  set_pin "$latest"
  validate && outcome=green || outcome=broken
fi

if [ "$outcome" = green ]; then
  if [ "$latest" = "$pinned" ]; then log "forced run: $latest still validates; nothing to land"; printf '%s\n' "$REPORT"; exit 0; fi
  land "$pinned" "$latest" "green" "auto-bumped"; exit $?
fi

report="$REPORTS/report-$latest.md"
{ echo "# Upkeep break: SolidCraft $pinned -> $latest ($(date -Iseconds))"; echo; echo "Worktree: $WT (pin already set to $latest)"; printf '%s\n' "$REPORT"; } > "$report"
log "BROKEN: report at $report"
[ "$latest" = "$pinned" ] && { log "forced run on the pinned release broke: the problem is ours, not upstream's"; }
if [ $session = 1 ]; then
  git -C "$WT" switch -q -C "upkeep/$latest"
  start_session "$report"
fi
exit 1
