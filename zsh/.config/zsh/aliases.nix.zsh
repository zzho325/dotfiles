alias cl='clear'
alias lc='clear'
alias reload='source ~/.zshrc'
alias tmux='tmux -2 -f "$XDG_CONFIG_HOME"/tmux/tmux.conf'
alias lock='xlock'
alias x='xdg-open &>/dev/null'
alias l='ls -l --color'
alias ll='ls -alh --color'
alias py='python3'
alias v='nvim'
alias vi='nvim'
alias vim='nvim'
alias cs1951a_venv='source ~/Development/datascience/cs1951a_venv/bin/activate'
alias sq='sqlite3'
alias wget='wget --hsts-file="$XDG_CACHE_HOME/wget-hsts" --show-progress'
alias lg='lazygit'
alias tmux-copy="wl-copy"
alias nv='cd $HOME/.config/nvim && nvim'
alias ck='~/cookie/target/debug/cookie'
alias treev='eza --tree --header --git --ignore-glob .DS_Store --icons --all'
pr() {
  if [ -n "$1" ]; then
    gh pr view "${1#\#}" --web
  else
    gh pr view --web
  fi
}
rebase-main() {
  git fetch origin main
  git branch -f main origin/main
  if [[ -n "$1" ]]; then
    git rebase --autostash --onto origin/main HEAD~"$1"
  else
    local n
    n=$(git rev-list --count --right-only --cherry-pick origin/main...HEAD)
    if [[ "$n" -eq 0 ]]; then
      echo "No unique commits to rebase"
      return 0
    fi
    echo "Rebasing $n commit(s) onto origin/main"
    git rebase --autostash --onto origin/main HEAD~"$n"
  fi
}

