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
# [ -f ~/.zsh_history ] && rm ~/.zsh_history # if override zsh history
[ -f ~/.zcompdump* ] && rm ~/.zcompdump*
# unnecessary with nvim
[ -f ~/.vim_history ] && rm ~/.vim_history
[ -f ~/.viminfo ] && rm ~/.viminfo
[ -f ~/.DS_Store ] && rm ~/.DS_Store

# Stow application configs
if command -v stow >/dev/null; then
  cd "$HOME/dotfiles"
  stow --adopt zsh nvim starship ghostty --target="$HOME"
  stow zsh nvim starship ghostty --target="$HOME"
  echo "✔ Stowed all dotfiles into $HOME"
else
  echo "⚠ stow not installed; skipping config stow"
fi

echo "✔ Bootstrap complete!"

