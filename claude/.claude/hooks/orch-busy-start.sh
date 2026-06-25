#!/bin/sh
# Wrapper: forwards Claude Code's UserPromptSubmit hook input to
# `orch busy start`, which writes a marker at
# $XDG_RUNTIME_DIR/orch/busy/<session_id>. See `orch busy --help`.
exec orch busy start
