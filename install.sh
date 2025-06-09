#!/bin/bash

echo "Linking Neovim config..."

if [ -e "$HOME/.config/nvim" ] || [ -L "$HOME/.config/nvim" ]; then
  echo "Removing existing ~/.config/nvim"
  rm -rf "$HOME/.config/nvim"
fi

ln -s "$HOME/.dotfiles/nvim" "$HOME/.config/nvim"
echo "✅ Neovim config linked successfully."

