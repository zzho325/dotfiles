# general
alias cl='clear'
alias lc='clear'
alias reload='source ~/.zshrc'
alias l='ls -l --color'
alias ll='ls -alh --color'
alias py='python3'
alias sq='sqlite3'
alias lg='lazygit'
alias treev='eza --tree --header --git --ignore-glob .DS_Store --icons --all'

# editors
alias v='nvim'
alias vi='nvim'
alias vim='nvim'
alias nv='cd $HOME/.config/nvim && nvim'

# tools
alias tmux='tmux -2 -f "$XDG_CONFIG_HOME"/tmux/tmux.conf'
alias tmux-copy="wl-copy"
alias lock='xlock'
alias x='xdg-open &>/dev/null'
alias wget='wget --hsts-file="$XDG_CACHE_HOME/wget-hsts" --show-progress'
alias ck='~/cookie/target/debug/cookie'
alias cs1951a_venv='source ~/Development/datascience/cs1951a_venv/bin/activate'

# git / github
openpr() {
  if [ -n "$1" ]; then
    gh pr view "${1#\#}" --web
  else
    gh pr view --web
  fi
}

pr() {
  gh pr list --search "is:open involves:@me" \
    --json number,title,author \
    --template '{{range .}}#{{.number}}	{{.author.login}}	{{.title}}{{"\n"}}{{end}}' \
    | fzf --prompt="PR> " \
      --preview='gh pr view {1} --comments' \
      --bind='enter:execute(gh pr view {1} --web)' \
      --bind='ctrl-a:execute(gh pr review {1} --approve)'
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
