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

# >>> conda initialize >>>
__conda_setup="$('/Users/ashley.zhou/miniconda3/bin/conda' 'shell.zsh' 'hook' 2> /dev/null)"
if [ $? -eq 0 ]; then
    eval "$__conda_setup"
else
    if [ -f "/Users/ashley.zhou/miniconda3/etc/profile.d/conda.sh" ]; then
        . "/Users/ashley.zhou/miniconda3/etc/profile.d/conda.sh"
    else
        export PATH="/Users/ashley.zhou/miniconda3/bin:$PATH"
    fi
fi
unset __conda_setup
# <<< conda initialize <<<

# >>> Google Cloud >>>
if [ -f '/Users/ashley.zhou/google-cloud-sdk/path.zsh.inc' ]; then . '/Users/ashley.zhou/google-cloud-sdk/path.zsh.inc'; fi

if [ -f '/Users/ashley.zhou/google-cloud-sdk/completion.zsh.inc' ]; then . '/Users/ashley.zhou/google-cloud-sdk/completion.zsh.inc'; fi
export PATH="$HOME/google-cloud-sdk/bin:$PATH"
# <<< Google Cloud >>>
