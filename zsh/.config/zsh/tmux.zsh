# update tmux window name: preexec sets to command, chpwd sets to dirname
if [[ -n "$TMUX" ]]; then
  preexec_tmux_rename() { tmux rename-window "${1%% *}" }
  chpwd_tmux_rename() { tmux rename-window "${PWD##*/}" }
  preexec_functions+=(preexec_tmux_rename)
  chpwd_functions+=(chpwd_tmux_rename)
fi
