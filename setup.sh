set -euo pipefail

# Install homebrew and run Brewfile
if ! command -v brew >/dev/null; then
  echo "→ Installing Homebrew…"
  /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
else
  echo "✔ Homebrew already installed, skipping"
fi

echo "→ Updating Homebrew…"
brew update
brew upgrade

# TODO: update Brewfile to only include cast for local env
echo "→ Running Brewfile…"
brew bundle install --file=./setup/Brewfile

echo "→ Homebrew cleanup..."
brew cleanup

# Remove unnecessary files if present in the home directory
echo "→ Removing legacy files…"
[ -f ~/.bash_history ] && rm ~/.bash_history
[ -f ~/.bash_functions ] && rm ~/.bash_functions
[ -f ~/.bashrc ] && rm ~/.bashrc
[ -f ~/.zsh_history ] && rm ~/.zsh_history # if override zsh history
[ -f ~/.zcompdump* ] && rm ~/.zcompdump*
# unnecessary with nvim
[ -f ~/.vim_history ] && rm ~/.vim_history
[ -f ~/.viminfo ] && rm ~/.viminfo
[ -f ~/.DS_Store ] && rm ~/.DS_Store

# Install fonts
echo "→ Installing fonts…"
find "$HOME/dotfiles/fonts" -type f \
  \( -iname '*.otf' -o -iname '*.ttf' -o -iname '*.ttc' -o -iname '*.otc' \) \
  -exec cp -f {} "$HOME/Library/Fonts"/ \;
echo "✔ Fonts installed"

# Stow application configs
if command -v stow >/dev/null; then
  cd "$HOME/dotfiles"
  stow --adopt --no-folding zsh nvim git jj starship ghostty zellij tmux worktrunk claude agents codex --target="$HOME"
  echo "✔ Stowed all dotfiles into $HOME"
else
  echo "⚠ stow not installed; skipping config stow"
fi

# Replace stow symlinks in skills/ with hardlinks (codex needs real files)
echo "→ Hardlinking skills…"
for f in "$HOME/.agents/skills"/*/SKILL.md; do
  [ -L "$f" ] || continue
  src=$(readlink -f "$f")
  rm "$f"
  ln "$src" "$f"
done

# Link claude and codex agents/skills to shared agent package
echo "→ Linking agents/skills…"
ln -sfn "$HOME/.agents/agents" "$HOME/.claude/agents"
ln -sfn "$HOME/.agents/skills" "$HOME/.claude/skills"
ln -sfn "$HOME/.agents/design" "$HOME/.claude/design"

# Merge local-only skills into the shared skills dir
if [ -d "$HOME/.agents/skills_local" ]; then
  for skill in "$HOME/.agents/skills_local"/*/; do
    name=$(basename "$skill")
    ln -sfn "$skill" "$HOME/.agents/skills/$name"
  done
fi
echo "✔ Agents/skills linked"

echo "✔ Bootstrap complete!"

