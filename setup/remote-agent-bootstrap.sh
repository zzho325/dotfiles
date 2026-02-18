#!/usr/bin/env bash
# Bootstrap dotfiles on a Column remote-agent Fargate container.
# Installs all tools to ~/bin (no sudo needed), then clones + stows dotfiles.
#
# Usage:
#   columnctl agent start --attach --entrypoint \
#     'curl -fsSL https://raw.githubusercontent.com/zzho325/dotfiles/main/setup/remote-agent-bootstrap.sh | bash; exec bash'
set -euo pipefail

log() { echo "[dotfiles] $(date '+%H:%M:%S') $*"; }

LOCAL_BIN="$HOME/bin"
DOTFILES="$HOME/dotfiles"
XDG_CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"
ARCH=$(uname -m)  # aarch64 or x86_64

mkdir -p "$LOCAL_BIN" "$XDG_CONFIG_HOME"
export PATH="$LOCAL_BIN:$PATH"

# ── 1. Install tools to userspace ────────────────────────────────────────────

# stow (perl script, just needs perl which is on ubuntu)
if ! command -v stow &>/dev/null; then
  log "Installing stow..."
  STOW_VERSION="2.4.1"
  curl -fsSL "https://ftp.gnu.org/gnu/stow/stow-${STOW_VERSION}.tar.gz" -o /tmp/stow.tar.gz
  tar xzf /tmp/stow.tar.gz -C /tmp
  cd /tmp/stow-${STOW_VERSION}
  ./configure --prefix="$HOME/.local" 2>/dev/null && make install 2>/dev/null
  ln -sf "$HOME/.local/bin/stow" "$LOCAL_BIN/stow"
  cd - >/dev/null
  rm -rf /tmp/stow*
fi

# starship
if ! command -v starship &>/dev/null; then
  log "Installing starship..."
  curl -fsSL https://starship.rs/install.sh | sh -s -- -y -b "$LOCAL_BIN" >/dev/null 2>&1
fi

# delta
if ! command -v delta &>/dev/null; then
  log "Installing delta..."
  DELTA_VERSION="0.18.2"
  if [ "$ARCH" = "aarch64" ]; then
    DELTA_ARCH="aarch64-unknown-linux-gnu"
  else
    DELTA_ARCH="x86_64-unknown-linux-gnu"
  fi
  curl -fsSL "https://github.com/dandavison/delta/releases/download/${DELTA_VERSION}/delta-${DELTA_VERSION}-${DELTA_ARCH}.tar.gz" \
    | tar xz -C /tmp
  mv "/tmp/delta-${DELTA_VERSION}-${DELTA_ARCH}/delta" "$LOCAL_BIN/delta"
  rm -rf "/tmp/delta-${DELTA_VERSION}-${DELTA_ARCH}"
fi

# fzf
if ! command -v fzf &>/dev/null; then
  log "Installing fzf..."
  FZF_VERSION="0.60.3"
  if [ "$ARCH" = "aarch64" ]; then
    FZF_ARCH="linux_arm64"
  else
    FZF_ARCH="linux_amd64"
  fi
  curl -fsSL "https://github.com/junegunn/fzf/releases/download/v${FZF_VERSION}/fzf-${FZF_VERSION}-${FZF_ARCH}.tar.gz" \
    | tar xz -C "$LOCAL_BIN"
fi

# neovim
if ! command -v nvim &>/dev/null; then
  log "Installing neovim..."
  if [ "$ARCH" = "aarch64" ]; then
    NVIM_URL="https://github.com/neovim/neovim/releases/download/stable/nvim-linux-arm64.tar.gz"
  else
    NVIM_URL="https://github.com/neovim/neovim/releases/download/stable/nvim-linux-x86_64.tar.gz"
  fi
  curl -fsSL "$NVIM_URL" | tar xz -C /tmp
  mv /tmp/nvim-linux-*/bin/nvim "$LOCAL_BIN/nvim"
  # also need runtime files for nvim to work
  mkdir -p "$HOME/.local/share"
  rm -rf "$HOME/.local/share/nvim-runtime"
  mv /tmp/nvim-linux-*/share/nvim "$HOME/.local/share/nvim-runtime"
  rm -rf /tmp/nvim-linux-*
fi

# direnv
if ! command -v direnv &>/dev/null; then
  log "Installing direnv..."
  DIRENV_ARCH=$( [ "$ARCH" = "aarch64" ] && echo "arm64" || echo "amd64" )
  curl -fsSL "https://github.com/direnv/direnv/releases/latest/download/direnv.linux-${DIRENV_ARCH}" -o "$LOCAL_BIN/direnv"
  chmod +x "$LOCAL_BIN/direnv"
fi

log "Tools installed to $LOCAL_BIN"

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

# ── 4. Create .zsh_local for remote-agent-specific overrides ─────────────────
cat > "$HOME/.zsh_local" << 'LOCALEOF'
# Remote agent overrides
export TERM=screen-256color
export PATH="$HOME/bin:$HOME/.local/bin:$PATH"

# neovim runtime
export VIMRUNTIME="$HOME/.local/share/nvim-runtime/runtime"

# direnv hook (skip if not installed)
if command -v direnv &>/dev/null; then
  eval "$(direnv hook zsh)"
fi

# tmux-copy fallback (no wl-copy on Fargate)
command -v wl-copy &>/dev/null || alias tmux-copy="cat > /dev/null"

# wt may not be available
command -v wt &>/dev/null || wt() { :; }
LOCALEOF

log "Done! Dotfiles bootstrapped. Run 'exec bash' or reattach to pick up changes."
