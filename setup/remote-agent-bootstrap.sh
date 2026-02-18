#!/usr/bin/env bash
# Bootstrap dotfiles on a remote-agent Fargate container.
# Designed to run as the `agent` user inside the tmux entrypoint.
#
# Usage (from local machine):
#   columnctl agent start --attach --entrypoint \
#     'curl -fsSL https://raw.githubusercontent.com/zzho325/dotfiles/main/setup/remote-agent-bootstrap.sh | bash'
set -euo pipefail

log() { echo "[dotfiles] $(date '+%H:%M:%S') $*"; }

DOTFILES="$HOME/dotfiles"
XDG_CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"

# ── 1. Install missing packages ──────────────────────────────────────────────
log "Installing packages..."
sudo apt-get update -qq
sudo apt-get install -y -qq --no-install-recommends \
  zsh stow fzf neovim delta direnv starship 2>/dev/null \
  || {
    # starship / delta aren't in default Ubuntu repos — install manually
    sudo apt-get install -y -qq --no-install-recommends \
      zsh stow fzf neovim direnv 2>/dev/null || true

    # starship
    if ! command -v starship &>/dev/null; then
      log "Installing starship..."
      curl -fsSL https://starship.rs/install.sh | sh -s -- -y >/dev/null 2>&1
    fi

    # delta (git-delta)
    if ! command -v delta &>/dev/null; then
      log "Installing delta..."
      DELTA_VERSION="0.18.2"
      ARCH=$(dpkg --print-architecture)
      curl -fsSL "https://github.com/dandavison/delta/releases/download/${DELTA_VERSION}/git-delta_${DELTA_VERSION}_${ARCH}.deb" \
        -o /tmp/delta.deb
      sudo dpkg -i /tmp/delta.deb && rm -f /tmp/delta.deb
    fi
  }

# ── 2. Clone dotfiles ────────────────────────────────────────────────────────
if [ -d "$DOTFILES/.git" ]; then
  log "Dotfiles already cloned, pulling latest..."
  git -C "$DOTFILES" pull --ff-only 2>/dev/null || true
else
  log "Cloning dotfiles..."
  git clone --depth=1 https://github.com/zzho325/dotfiles.git "$DOTFILES"
fi

# ── 3. Stow configs ──────────────────────────────────────────────────────────
log "Stowing configs..."
mkdir -p "$XDG_CONFIG_HOME"

# Remove conflicting defaults before stowing
rm -f "$HOME/.zshrc" "$HOME/.bashrc" "$HOME/.gitconfig" 2>/dev/null || true

cd "$DOTFILES"
stow --adopt --no-folding -d "$DOTFILES" -t "$HOME" zsh tmux starship git nvim

# Restore any files that --adopt may have overwritten in the repo
git -C "$DOTFILES" checkout -- . 2>/dev/null || true

# ── 4. Set zsh as default shell ──────────────────────────────────────────────
ZSH_PATH="$(command -v zsh)"
if [ -n "$ZSH_PATH" ]; then
  log "Setting zsh as default shell..."
  sudo chsh -s "$ZSH_PATH" agent 2>/dev/null || true
fi

# ── 5. Create .zsh_local for remote-agent-specific overrides ─────────────────
cat > "$HOME/.zsh_local" << 'LOCALEOF'
# Remote agent overrides
export TERM=screen-256color

# direnv may not be available
command -v direnv &>/dev/null || eval 'direnv() { :; }'

# tmux-copy fallback (no wl-copy on Fargate)
command -v wl-copy &>/dev/null || alias tmux-copy="cat > /dev/null"

# No homebrew on Linux
command -v brew &>/dev/null || true
LOCALEOF

log "Done! Dotfiles bootstrapped."
log "Run 'exec zsh' or reattach tmux to pick up changes."
