#!/bin/sh
# orch busy detection: remove the marker when a Claude turn ends
# (Stop hook) or session terminates (SessionEnd hook).
#
# See orch-busy-start.sh for the marker contract.

set -eu

ORCH_HOOK_JSON=$(cat)
export ORCH_HOOK_JSON

python3 <<'PY'
import json, os, pathlib, sys
debug = os.environ.get("ORCH_HOOK_DEBUG")
try:
    d = json.loads(os.environ.get("ORCH_HOOK_JSON", ""))
except Exception as e:
    if debug:
        sys.stderr.write(f"orch-busy-stop: bad JSON: {e}\n")
    sys.exit(0)
sid = d.get("session_id")
if not sid:
    if debug:
        sys.stderr.write("orch-busy-stop: missing session_id\n")
    sys.exit(0)
runtime = os.environ.get("XDG_RUNTIME_DIR", "/tmp")
p = pathlib.Path(runtime) / "orch" / "busy" / sid
try:
    p.unlink()
except FileNotFoundError:
    pass
PY
