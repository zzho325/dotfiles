# general
alias cl='clear'
alias lc='clear'
alias reload='source ~/.zshrc'
alias l='ls -l --color'
alias ll='ls -alh --color'
alias py='python3'
alias sq='sqlite3'
alias lg='lazygit'
alias jjs='jj log -r "::@ ~ ::main" --no-graph'

jj-init() {
  local gitdir=$(git rev-parse --git-dir 2>/dev/null)
  if [[ -z "$gitdir" ]]; then
    echo "Not in a git repo"
    return 1
  fi
  if [[ -d .jj ]]; then
    echo "jj already initialized"
    return 0
  fi
  jj git init --git-repo="$gitdir"
}
alias treev='eza --tree --header --git --ignore-glob .DS_Store --icons --all'

# editors
alias v='nvim'
alias vi='nvim'
alias vim='nvim'
alias nv='cd $HOME/.config/nvim && nvim'
alias vg='rg --line-number --no-heading . | fzf --delimiter=: --preview "bat --color=always --highlight-line {2} {1}" | awk -F: '\''{print "+"$2, $1}'\'' | xargs nvim'

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
  if [[ -d .jj ]]; then
    jj git fetch
    jj git export
    git branch -f main origin/main 2>/dev/null
    local root
    root=$(jj log -r 'roots(::@ & ~::main)' --no-graph -T 'change_id.shortest()' --limit 1)
    if [[ -z "$root" ]]; then
      echo "No commits to rebase"
      return 0
    fi
    echo "Rebasing stack from $root onto main"
    jj rebase -s "$root" -d main
  else
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
  fi
}

