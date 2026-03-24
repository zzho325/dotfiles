# Dotfiles

Bootstrap dev environment. Managed with [GNU Stow](https://www.gnu.org/software/stow/).

## Structure

Each top-level directory is a **stow package** — its contents mirror `$HOME`:

```
dotfiles/
├── agents/          → ~/.agents/        # AI agent configs, skills, design docs
│   └── .agents/
│       ├── agents/                      # Agent definitions (AGENTS.md)
│       ├── skills/                      # Claude Code / Codex skills (SKILL.md each)
│       │   ├── orderlint/
│       │   ├── reviewbot/
│       │   ├── remote-session/
│       │   └── ...
│       ├── skills_local/                # Machine-local skills (not stowed)
│       └── design/                      # Design docs
├── claude/          → ~/.claude/        # Claude Code settings, hooks, keybindings
│   └── .claude/
│       ├── settings.json
│       ├── hooks/
│       └── CLAUDE.md
├── codex/           → ~/.codex/         # OpenAI Codex config
├── ghostty/         → ~/.config/ghostty/
├── git/             → ~/.config/git/
├── nvim/            → ~/.config/nvim/
├── orch/            → ~/.orch/          # Orchestrator (Rust)
├── starship/        → ~/.config/starship/
├── tmux/            → ~/.config/tmux/
├── tools/                               # Custom dev tools (not stowed)
│   └── orderlint/                       # Go linter for function ordering
├── worktrunk/       → ~/.config/worktrunk/
├── zellij/          → ~/.config/zellij/
├── zsh/             → ~/.config/zsh/
└── setup/                               # Brewfile, bootstrap helpers
```

## How stow works

`stow <package>` symlinks the package's contents into `$HOME`:

```
dotfiles/git/.config/git/config  →  ~/.config/git/config
dotfiles/agents/.agents/skills/orderlint/  →  ~/.agents/skills/orderlint
```

### Skills linkage

Skills live in `agents/.agents/skills/` and are stowed to `~/.agents/skills/`.
`setup.sh` then symlinks `~/.claude/skills → ~/.agents/skills` so Claude Code
and Codex share the same skill set.

Machine-local skills (credentials, env-specific) go in `~/.agents/skills_local/`
and are symlinked into `~/.agents/skills/` by `setup.sh` — they stay out of git.

### Adding a new skill

1. Create `agents/.agents/skills/<name>/SKILL.md`
2. Run `stow -R agents`
3. The skill appears in `~/.agents/skills/<name>` and is visible to Claude Code

### Adopting external changes

If something was modified outside stow (e.g., Claude Code edited a skill):

```bash
stow --adopt agents    # pulls live files into repo
stow -R agents         # re-creates symlinks
```

## Setup

```bash
./setup.sh
```

Installs Homebrew deps, stows all packages, and wires up skill symlinks.
