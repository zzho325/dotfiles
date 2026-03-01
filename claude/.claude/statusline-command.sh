#!/bin/sh
# Claude Code status line — mirrors Starship "┌─> / └─>" style

input=$(cat)

cwd=$(echo "$input" | jq -r '.workspace.current_dir // .cwd // ""')
model=$(echo "$input" | jq -r '.model.display_name // ""')
used=$(echo "$input" | jq -r '.context_window.used_percentage // empty')
session=$(echo "$input" | jq -r '.session_name // empty')
cost=$(echo "$input" | jq -r '.cost.total_cost_usd // empty')
sid=$(echo "$input" | jq -r '.session_id // empty')

# Log per-session cost on every tick.
# total_cost_usd is cumulative per process. We track the last-seen
# process cost in /tmp and add each delta to the log entry, so
# resumes (new process, same session) accumulate correctly.
cost_log="$HOME/.claude/session-cost-log.json"
if [ -n "$sid" ] && [ -n "$cost" ]; then
    prev_file="/tmp/claude-cost-prev-${sid}"

    # On first tick (or after resume), seed prev with current cost
    if [ ! -f "$prev_file" ]; then
        echo "$cost" > "$prev_file"
    fi
    prev_cost=$(cat "$prev_file")
    delta=$(awk "BEGIN{print $cost - $prev_cost}")
    echo "$cost" > "$prev_file"

    ts=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    [ ! -s "$cost_log" ] && echo '{}' > "$cost_log"
    jq --arg sid "$sid" \
       --argjson delta "$delta" \
       --arg model "$model" \
       --arg ts "$ts" \
       --arg cwd "$cwd" \
       '.[$sid].cost_usd = ((.[$sid].cost_usd // 0) + $delta)
        | .[$sid].model = $model
        | .[$sid].last_seen = $ts
        | .[$sid].cwd = $cwd' \
       "$cost_log" > "${cost_log}.tmp" 2>/dev/null \
    && mv "${cost_log}.tmp" "$cost_log" \
    || rm -f "${cost_log}.tmp"
fi

# Shorten cwd: replace $HOME with ~
home="$HOME"
short_cwd=$(echo "$cwd" | sed "s|^$home|~|")

# Git branch (skip optional lock)
git_branch=""
if git -C "$cwd" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    git_branch=$(git -C "$cwd" symbolic-ref --short HEAD 2>/dev/null \
        || git -C "$cwd" rev-parse --short HEAD 2>/dev/null)
fi

# ANSI colors (will be dimmed by the terminal)
bold_green="\033[1;32m"
cyan="\033[0;36m"
yellow="\033[0;33m"
magenta="\033[0;35m"
reset="\033[0m"

# Build top line parts
parts="${bold_green}${short_cwd}${reset}"

if [ -n "$git_branch" ]; then
    parts="${parts}  ${cyan}${git_branch}${reset}"
fi

if [ -n "$model" ]; then
    parts="${parts}  ${magenta}${model}${reset}"
fi

if [ -n "$used" ]; then
    used_int=$(printf "%.0f" "$used")
    parts="${parts}  ${yellow}ctx:${used_int}%${reset}"
fi

if [ -n "$sid" ] && [ -s "$cost_log" ]; then
    sc=$(jq -r --arg sid "$sid" '.[$sid].cost_usd // empty' "$cost_log" 2>/dev/null)
    if [ -n "$sc" ]; then
        cost_fmt=$(printf "%.2f" "$sc")
        parts="${parts}  ${yellow}\$${cost_fmt}${reset}"
    fi
fi

if [ -n "$session" ]; then
    parts="${parts}  ${cyan}[${session}]${reset}"
fi

printf "%b\n" "$parts"
