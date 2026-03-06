export FZF_COMPLETION_TRIGGER="**"

_fzf_base_opts='--height 40% --layout=reverse --border=rounded'
_fzf_dark='--color=bg:#191724,bg+:#26233a,fg:#e0def4,fg+:#e0def4,hl:#c4a7e7,hl+:#c4a7e7,border:#403d52,prompt:#9ccfd8,pointer:#eb6f92,info:#6e6a86,header:#31748f'
_fzf_light='--color=bg:#faf4ed,bg+:#f2e9e1,fg:#575279,fg+:#575279,hl:#907aa9,hl+:#907aa9,border:#dfdad9,prompt:#56949f,pointer:#b4637a,info:#9893a5,header:#286983'

_update_theme() {
  if [[ "$(defaults read -g AppleInterfaceStyle 2>/dev/null)" == "Dark" ]]; then
    export FZF_DEFAULT_OPTS="$_fzf_base_opts $_fzf_dark"
    export BAT_THEME="rose-pine"
  else
    export FZF_DEFAULT_OPTS="$_fzf_base_opts $_fzf_light"
    export BAT_THEME="rose-pine-dawn"
  fi
}
precmd_functions+=(_update_theme)
_update_theme
