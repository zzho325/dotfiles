# update tmux window name: auto-rename tracks running command, precmd resets to dirname
if [[ -n "$TMUX" ]]; then
  preexec_tmux_rename() { tmux rename-window "${1%% *}" }
  precmd_tmux_rename() { tmux rename-window "${PWD##*/}" }
  preexec_functions+=(preexec_tmux_rename)
  precmd_functions+=(precmd_tmux_rename)
fi
