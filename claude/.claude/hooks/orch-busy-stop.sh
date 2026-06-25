#!/bin/sh
# Wrapper: forwards Claude Code's Stop / SessionEnd hook input to
# `orch busy stop`, which removes the marker written by
# `orch-busy-start.sh`. See `orch busy --help`.
exec orch busy stop
