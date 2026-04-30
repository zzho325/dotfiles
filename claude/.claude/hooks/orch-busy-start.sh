#!/bin/sh
# orch busy detection: write a marker when a Claude turn starts
# (UserPromptSubmit hook). Marker is removed by orch-busy-stop.sh on Stop.
#
# Marker path: $XDG_RUNTIME_DIR/orch/busy/<session_id>
# (falls back to /tmp/orch/busy/ if XDG_RUNTIME_DIR unset)
#
# Hook input is JSON on stdin: { "session_id": "...", "cwd": "..." }
# Exit 0 fast and write nothing to stdout — UserPromptSubmit blocks.

set -eu

# Capture stdin into env var so the heredoc-loaded python script can read it
# (a `python3 - <<EOF` pattern would consume stdin as the source itself).
ORCH_HOOK_JSON=$(cat)
export ORCH_HOOK_JSON

python3 <<'PY'
import json, os, pathlib, sys, time
try:
    d = json.loads(os.environ.get("ORCH_HOOK_JSON", ""))
except Exception:
    sys.exit(0)
sid = d.get("session_id")
if not sid:
    sys.exit(0)
runtime = os.environ.get("XDG_RUNTIME_DIR", "/tmp")
p = pathlib.Path(runtime) / "orch" / "busy" / sid
p.parent.mkdir(parents=True, exist_ok=True)
p.write_text(json.dumps({
    "cwd": d.get("cwd", ""),
    "started_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    "pid": os.getpid(),
}))
PY
