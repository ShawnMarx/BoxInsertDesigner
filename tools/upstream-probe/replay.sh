#!/bin/bash
# Replay: keep a fork branch current by merging each upstream release tag.
set -u
S=$(cd "$(dirname "$0")" && pwd)
git -c advice.detachedHead=false checkout -q -B fork v0.18.0
python3 -I "$S/apply_fork.py" . >/dev/null
git add -A && git -c user.name=t -c user.email=t@t commit -qm "fork: insert UI" 
for T in $(git tag | grep -E '^v0\.(1[89]|2[0-9])\.[0-9]+$' | sort -V | sed 1d); do
  files=$(git diff --stat HEAD~0 $T -- crates/ui-egui/src/workspace.rs crates/ui-egui/src/lib.rs crates/ui-egui/src/viewport.rs | tail -1)
  if git -c user.name=t -c user.email=t@t merge -q --no-edit $T >/dev/null 2>&1; then
    hits=$(grep -c 'GAME BOX\|insert_hud' crates/ui-egui/src/workspace.rs crates/ui-egui/src/lib.rs crates/ui-egui/src/viewport.rs | tr '\n' ' ')
    echo "$T  merge=CLEAN     fork-lines: $hits"
  else
    cf=$(git diff --name-only --diff-filter=U | sed 's#crates/ui-egui/src/##' | tr '\n' ' ')
    n=$(git diff | grep -c '^[+ ]<<<<<<<')
    git merge --abort
    git -c advice.detachedHead=false checkout -q -B fork $T
    r=$(python3 -I "$S/apply_fork.py" .)
    git add -A && git -c user.name=t -c user.email=t@t commit -qm "fork: re-apply on $T"
    echo "$T  merge=CONFLICT ($n hunks in: $cf)  re-apply: $r"
  fi
done
