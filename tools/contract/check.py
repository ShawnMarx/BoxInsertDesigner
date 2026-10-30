#!/usr/bin/env python3
"""Command contract test: the SolidCraft commands we call, and their params, per release.

    tools/contract/check.py <solidcraft-cli> record   # write contract/<version>.json
    tools/contract/check.py <solidcraft-cli>          # diff the CLI against the recorded pin

Exit 1 when a used command is missing or its params text changed.
"""
import json, pathlib, subprocess, sys

here = pathlib.Path(__file__).parent
cli = sys.argv[1]
used = [l.strip() for l in (here / "used.txt").read_text().splitlines() if l.strip()]
version = subprocess.run([cli, "--version"], capture_output=True, text=True, check=True).stdout.split()[-1]
cmds = {c["id"]: c["params"] for c in json.loads(subprocess.run([cli, "commands"], capture_output=True, text=True, check=True).stdout)}
now = {u: cmds.get(u) for u in used}

if sys.argv[2:] == ["record"]:
    (here / f"solidcraft-{version}.json").write_text(json.dumps(now, indent=2, ensure_ascii=False) + "\n")
    print(f"recorded {len(used)} commands for {version}")
    sys.exit(0)

pins = sorted(here.glob("solidcraft-*.json"))
base = json.loads(pins[-1].read_text())
bad = 0
for u in used:
    if now[u] is None:
        print(f"MISSING  {u}"); bad += 1
    elif base.get(u) != now[u]:
        print(f"CHANGED  {u}\n  was: {base.get(u)}\n  now: {now[u]}"); bad += 1
print(f"{version} vs {pins[-1].name}: {len(used) - bad}/{len(used)} unchanged")
sys.exit(1 if bad else 0)
