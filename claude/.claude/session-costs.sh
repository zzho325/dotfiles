#!/bin/bash
# Shows per-session costs from the statusline cost log.
#
# Usage:
#   ~/.claude/session-costs.sh              # all sessions
#   ~/.claude/session-costs.sh --days 7     # last 7 days
#   ~/.claude/session-costs.sh --summary    # daily summary

set -euo pipefail

COST_LOG="$HOME/.claude/session-cost-log.json"
DAYS=0
SUMMARY=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --days)    DAYS="$2"; shift 2 ;;
        --summary) SUMMARY=true; shift ;;
        *)         echo "Unknown flag: $1"; exit 1 ;;
    esac
done

if [[ "$DAYS" -gt 0 ]]; then
    CUTOFF=$(date -v-"${DAYS}"d -u +%Y-%m-%dT%H:%M:%SZ 2>/dev/null \
        || date -u -d "$DAYS days ago" +%Y-%m-%dT%H:%M:%SZ)
else
    CUTOFF="1970-01-01T00:00:00Z"
fi

if [ ! -s "$COST_LOG" ]; then
    echo "No cost data yet. Costs are logged by the statusline."
    exit 0
fi

jq -r --arg cutoff "$CUTOFF" --argjson summary "$SUMMARY" '
to_entries
| map({
    session: .key,
    cost: .value.cost_usd,
    model: .value.model,
    cwd: (.value.cwd | split("/") | last),
    date: .value.last_seen[0:10],
    ts: .value.last_seen,
  })
| map(select(.ts >= $cutoff))
| sort_by(.ts)
| if $summary then
    group_by(.date)
    | map({
        date: .[0].date,
        sessions: length,
        cost: (map(.cost) | add),
      })
    | (["DATE", "SESSIONS", "COST"] | @tsv),
      (.[] | [
        .date,
        .sessions,
        ("$" + ((.cost * 100 | round) / 100 | tostring))
      ] | @tsv),
      "",
      ("TOTAL: $" + (((map(.cost) | add) * 100 | round) / 100 | tostring))
  else
    (["DATE", "PROJECT", "MODEL", "COST"] | @tsv),
    (.[] | [
      .ts[0:16],
      .cwd,
      .model,
      ("$" + ((.cost * 100 | round) / 100 | tostring))
    ] | @tsv)
  end
' "$COST_LOG"
