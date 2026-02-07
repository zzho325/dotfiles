export EDITOR='nvim'
export XDG_CONFIG_HOME="$HOME/.config"
export XDG_CACHE_HOME="$HOME/.cache"
export XDG_DATA_HOME="$HOME/.local/share"

export DOTFILES_LOCATION="$HOME/dotfiles"

export LESSHISTFILE="-"

export FZF_COMPLETION_TRIGGER="**"
export FZF_DEFAULT_OPTS='--height 40% --layout=reverse --border'

eval "$(direnv hook zsh)"

export PATH="$HOME/go/bin:$PATH"
export HISTFILE="$XDG_CONFIG_HOME/zsh/.zsh_history"
export HISTSIZE=999999999
export SAVEHIST=$HISTSIZE
setopt share_history          # share across all running shells
setopt inc_append_history     # append each command immediately
setopt hist_ignore_all_dups   # don’t record duplicates

eval "$(starship init zsh)"

# load nix Zsh snippets
for f in $XDG_CONFIG_HOME/zsh/*.nix.zsh; do
  source $f
done

# set to beam cursor
print -n "\e[6 q"

bindkey -e

# “history-search” on up/down
autoload -U compinit && compinit
autoload -U up-line-or-beginning-search
autoload -U down-line-or-beginning-search
zle -N up-line-or-beginning-search
zle -N down-line-or-beginning-search
bindkey '^[[A' up-line-or-beginning-search
bindkey '^[[B' down-line-or-beginning-search
# bindkey "${terminfo[kcuu1]}" up-line-or-beginning-search
# bindkey "${terminfo[kcud1]}" down-line-or-beginning-search

# fix delete char for tmux and zellij
bindkey '^?' backward-delete-char
bindkey '^H' backward-delete-char

# fc
autoload edit-command-line
zle -N edit-command-line
bindkey "^X^E" edit-command-line

# wt
if command -v wt >/dev/null 2>&1; then eval "$(command wt config shell init zsh)"; fi

# local
if [[ -r "$HOME/.zsh_local" ]]; then
  source "$HOME/.zsh_local"
fi
