#!/usr/bin/env bash
# Bootstrap dotfiles on a Column remote-agent Fargate container.
# Runs as the `agent` user inside the tmux entrypoint.
#
# Prerequisites: zsh, stow, neovim, starship, delta, fzf, direnv
# must be pre-installed in the Docker image.
#
# Usage:
#   columnctl agent start --attach --entrypoint \
#     'curl -fsSL https://raw.githubusercontent.com/zzho325/dotfiles/main/setup/remote-agent-bootstrap.sh | bash; exec zsh'
set -euo pipefail

log() { echo "[dotfiles] $(date '+%H:%M:%S') $*"; }

DOTFILES="$HOME/dotfiles"
XDG_CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"

# ── 1. Clone dotfiles ────────────────────────────────────────────────────────
if [ -d "$DOTFILES/.git" ]; then
  log "Dotfiles already cloned, pulling latest..."
  git -C "$DOTFILES" pull --ff-only 2>/dev/null || true
else
  log "Cloning dotfiles..."
  git clone --depth=1 https://github.com/zzho325/dotfiles.git "$DOTFILES"
fi

# ── 2. Stow configs ──────────────────────────────────────────────────────────
log "Stowing configs..."
mkdir -p "$XDG_CONFIG_HOME"

# Remove conflicting defaults before stowing
rm -f "$HOME/.zshrc" "$HOME/.bashrc" "$HOME/.gitconfig" 2>/dev/null || true

cd "$DOTFILES"
for pkg in zsh tmux starship git nvim; do
  if [ -d "$DOTFILES/$pkg" ]; then
    stow --adopt --no-folding -d "$DOTFILES" -t "$HOME" "$pkg" 2>/dev/null || true
  fi
done

# Restore any files that --adopt may have overwritten in the repo
git -C "$DOTFILES" checkout -- . 2>/dev/null || true

# ── 3. Create .zsh_local for remote-agent-specific overrides ─────────────────
cat > "$HOME/.zsh_local" << 'LOCALEOF'
# Remote agent overrides
export TERM=screen-256color

# direnv hook (skip if not installed)
if command -v direnv &>/dev/null; then
  eval "$(direnv hook zsh)"
fi

# tmux-copy fallback (no wl-copy on Fargate)
command -v wl-copy &>/dev/null || alias tmux-copy="cat > /dev/null"

# wt may not be available
command -v wt &>/dev/null || wt() { :; }
LOCALEOF

log "Done! Dotfiles bootstrapped."
