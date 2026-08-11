//! Three-pane TUI: task list, right-side detail tabs, and run log.
//!
//! Focus model:
//!
//! ```text
//! ┌─────────────────┬──────────────────────────────────────┐
//! │ tasks list      │ Overview · PRs · Panes               │
//! │  #1 task-foo  · │ ─────────────────────────────────── │
//! │  #2 task-bar  ⚡ │ <selected tab content>              │
//! │  #3 task-baz  ✓ │                                      │
//! │                 ├──────────────────────────────────────┤
//! │                 │ log: latest run output, wrapped      │
//! └─────────────────┴──────────────────────────────────────┘
//! ```

#![allow(dead_code)] // Some bindings stubbed for Phase 4+.

use std::{
    collections::{HashMap, HashSet},
    io::{self, stdout},
    process::{Command, Stdio},
    sync::OnceLock,
    time::{Duration, Instant},
};

use crossterm::{
    ExecutableCommand,
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    prelude::*,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};

use crate::cache;
use crate::state::{self, TaskStatus, load_tmux_sessions};
use crate::store::{self, DesiredState, TaskRecord};

const FAST_TICK: Duration = Duration::from_secs(2);

#[derive(Clone, Copy)]
struct Palette {
    text: Color,
    subtle: Color,
    muted: Color,
    love: Color,
    gold: Color,
    pine: Color,
    foam: Color,
    iris: Color,
    highlight_low: Color,
    diff_add: Color,
    diff_del: Color,
}

const LIGHT_PALETTE: Palette = Palette {
    text: Color::Rgb(0x57, 0x52, 0x79),
    subtle: Color::Rgb(0x79, 0x75, 0x93),
    muted: Color::Rgb(0x98, 0x93, 0xa5),
    love: Color::Rgb(0xb4, 0x63, 0x7a),
    gold: Color::Rgb(0xea, 0x9d, 0x34),
    pine: Color::Rgb(0x28, 0x69, 0x83),
    foam: Color::Rgb(0x56, 0x94, 0x9f),
    iris: Color::Rgb(0x90, 0x7a, 0xa9),
    highlight_low: Color::Rgb(0xf4, 0xed, 0xe8),
    diff_add: Color::Rgb(0xea, 0xf0, 0xe2),
    diff_del: Color::Rgb(0xf6, 0xe2, 0xe2),
};

const DARK_PALETTE: Palette = Palette {
    text: Color::Rgb(0xe0, 0xde, 0xf4),
    subtle: Color::Rgb(0x90, 0x8c, 0xaa),
    muted: Color::Rgb(0x6e, 0x6a, 0x86),
    love: Color::Rgb(0xeb, 0x6f, 0x92),
    gold: Color::Rgb(0xf6, 0xc1, 0x77),
    pine: Color::Rgb(0x31, 0x74, 0x8f),
    foam: Color::Rgb(0x9c, 0xcf, 0xd8),
    iris: Color::Rgb(0xc4, 0xa7, 0xe7),
    highlight_low: Color::Rgb(0x26, 0x23, 0x3a),
    diff_add: Color::Rgb(0x1f, 0x3a, 0x33),
    diff_del: Color::Rgb(0x3b, 0x25, 0x30),
};

fn palette() -> &'static Palette {
    static PALETTE: OnceLock<Palette> = OnceLock::new();
    PALETTE.get_or_init(|| {
        if macos_dark_mode() {
            DARK_PALETTE
        } else {
            LIGHT_PALETTE
        }
    })
}

fn macos_dark_mode() -> bool {
    Command::new("defaults")
        .args(["read", "-g", "AppleInterfaceStyle"])
        .output()
        .map(|output| output.status.success() && output.stdout.starts_with(b"Dark"))
        .unwrap_or(false)
}

// Layout constants.
const LIST_WIDTH: u16 = 34;
const SEPARATOR_WIDTH: u16 = 1;
const TAB_BAR_HEIGHT: u16 = 2; // tabs row + divider
const LOG_HEIGHT_RATIO: u16 = 35; // percent of right workspace
const HELP_OVERLAY_WIDTH: u16 = 60;
const HELP_OVERLAY_HEIGHT: u16 = 25;

// State.

/// Focus is a two-state toggle. The Log is a passive viewer — always
/// scrolled via global Ctrl-U/Ctrl-D/`<`/`>` regardless of focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    List,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Overview,
    Prs,
    Panes,
}

impl Tab {
    fn label(self) -> &'static str {
        match self {
            Tab::Overview => "Overview",
            Tab::Prs => "PRs",
            Tab::Panes => "Panes",
        }
    }

    fn next(self) -> Self {
        match self {
            Tab::Overview => Tab::Prs,
            Tab::Prs => Tab::Panes,
            Tab::Panes => Tab::Overview,
        }
    }

    fn prev(self) -> Self {
        match self {
            Tab::Overview => Tab::Panes,
            Tab::Prs => Tab::Overview,
            Tab::Panes => Tab::Prs,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TmuxPaneInfo {
    pub id: String,
    pub session: String,
    pub command: String,
    pub active: bool,
}

#[derive(Debug, Clone)]
pub struct TaskView {
    pub name: String,
    pub record: TaskRecord,
    pub status: TaskStatus,
    pub prs: Vec<state::PrData>,
    pub panes: Vec<TmuxPaneInfo>,
}

impl TaskView {
    pub fn id(&self) -> store::TaskId {
        self.record.id
    }

    pub fn drift(&self) -> bool {
        self.record.drift.any()
    }
}

#[derive(Clone)]
pub struct LogPane {
    pub run_id: Option<String>,
    pub lines: Vec<String>,
    /// Visual-row offset from top (after wrap).
    pub scroll: usize,
    /// True when scroll is at the bottom; new lines auto-scroll to keep
    /// it pinned. Toggles to false when the user scrolls up.
    pub follow_bottom: bool,
    pub last_len: u64,
    pub finished: bool,
}

impl Default for LogPane {
    fn default() -> Self {
        Self {
            run_id: None,
            lines: Vec::new(),
            scroll: 0,
            follow_bottom: true,
            last_len: 0,
            finished: false,
        }
    }
}

/// PR tab sub-state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrView {
    List {
        /// 0 = no cursor (no PRs linked, or freshly entered without
        /// cursor anchored).
        cursor_number: u32,
    },
    Detail {
        number: u32,
        focus: PrDetailFocus,
        /// Index into `CachedPrDiff.files`.
        file_cursor: usize,
        /// Visual-row offset into the diff body (sticky-margin scroll).
        scroll: u16,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrDetailFocus {
    Files,
    Diff,
}

impl Default for PrView {
    fn default() -> Self {
        PrView::List { cursor_number: 0 }
    }
}

#[derive(Clone)]
pub struct App {
    pub tasks: Vec<TaskView>,
    pub selected: usize,
    pub focus: Pane,
    pub detail_tab: Tab,
    /// Pane selected within the Panes tab.
    pub panes_selected: usize,
    /// PR tab sub-state.
    pub pr_view: PrView,
    /// Per-PR persisted detail-view position. Esc-out saves into this
    /// map, drilling back into the same PR restores. Only `(file_cursor,
    /// scroll)` survive; `focus` resets to `Files`.
    pub pr_detail_state: HashMap<u32, (usize, u16)>,
    pub log: LogPane,
    pub show_activity: bool,
    pub show_help: bool,
    pub daemon_alive: bool,
    pub last_fast: Instant,
    pub should_quit: bool,
    pub message_input: Option<String>,
    pub read_runs: HashSet<String>,
    pub last_run_count: usize,
    /// Transient operation feedback rendered near the bottom.
    /// Cleared on next non-toast key press.
    pub toast: Option<String>,
    /// Skip live IO during tests.
    pub readonly: bool,
}

impl App {
    pub fn new() -> Self {
        let tasks = Self::load_tasks();
        let last_run_count = crate::runs::list_runs(100).len();
        let daemon_alive = crate::cache::is_daemon_alive();

        let mut app = Self {
            tasks,
            selected: 0,
            focus: Pane::List,
            detail_tab: Tab::Overview,
            panes_selected: 0,
            pr_view: PrView::default(),
            pr_detail_state: HashMap::new(),
            log: LogPane::default(),
            show_activity: false,
            show_help: false,
            daemon_alive,
            last_fast: Instant::now(),
            should_quit: false,
            message_input: None,
            read_runs: HashSet::new(),
            last_run_count,
            toast: None,
            readonly: false,
        };
        if let Some(idx) = current_task_index(&app.tasks) {
            app.selected = idx;
        }
        app.open_latest_run();
        app
    }

    fn load_tasks() -> Vec<TaskView> {
        let status_cache = crate::cache::read_status();
        let pr_cache = crate::cache::read_prs();
        let daemon_alive = crate::cache::is_daemon_alive();

        let live_sessions = if daemon_alive {
            None
        } else {
            Some(load_tmux_sessions())
        };

        let store = store::Store::default();
        let Some(registry) = store.load_registry() else {
            return Vec::new();
        };

        registry
            .open_order
            .iter()
            .filter_map(|id| store.load_record(*id))
            .filter(|r| r.desired_state != DesiredState::Closed)
            .map(|record| {
                let name = record.slug.clone();
                let status = if daemon_alive {
                    status_cache
                        .tasks
                        .get(&name)
                        .map(|ct| status_from_str(&ct.status))
                        .unwrap_or(TaskStatus::Idle)
                } else if let Some(sessions) = &live_sessions {
                    state::derive_status(&record, sessions, state::busy_stale_secs())
                } else {
                    TaskStatus::Idle
                };

                let prs: Vec<state::PrData> = record
                    .links
                    .prs
                    .iter()
                    .map(|p| {
                        pr_cache
                            .prs
                            .get(&p.number)
                            .map(|cp| cp.to_pr_data())
                            .unwrap_or(state::PrData {
                                number: p.number,
                                ..Default::default()
                            })
                    })
                    .collect();

                let panes = panes_for_session(&record.tmux.session_name);
                TaskView {
                    name,
                    record,
                    status,
                    prs,
                    panes,
                }
            })
            .collect()
    }

    fn open_latest_run(&mut self) {
        if self.readonly {
            return;
        }
        let runs = crate::runs::list_runs(50);
        let run = runs
            .iter()
            .find(|r| !self.read_runs.contains(&r.id))
            .or(runs.first())
            .cloned();
        if let Some(run) = run {
            self.open_run(&run);
        }
    }

    fn open_run(&mut self, run: &crate::runs::RunMeta) {
        let content = crate::runs::read_output(&run.id);
        let last_len = content.len() as u64;
        let lines: Vec<String> = content.lines().map(String::from).collect();
        self.log = LogPane {
            run_id: Some(run.id.clone()),
            lines,
            scroll: 0,
            follow_bottom: true,
            last_len,
            finished: run.finished_at.is_some(),
        };
    }

    fn refresh_log(&mut self) {
        if self.readonly {
            return;
        }
        let Some(run) = crate::runs::list_runs(1).into_iter().next() else {
            return;
        };
        if self.log.run_id.as_deref() != Some(run.id.as_str()) {
            self.open_run(&run);
            return;
        }

        self.log.finished = run.finished_at.is_some();
        let run_id = run.id;
        let cur_len = crate::runs::output_len(&run_id);
        if cur_len == self.log.last_len {
            return;
        }
        self.log.last_len = cur_len;
        let content = crate::runs::read_output(&run_id);
        let was_following = self.log.follow_bottom;
        self.log.lines = content.lines().map(String::from).collect();
        if was_following {
            self.log.scroll = usize::MAX; // pin to bottom; render clamps.
        }
    }

    fn refresh_status(&mut self) {
        if self.readonly {
            return;
        }
        let next_tasks = Self::load_tasks();
        let prev_name = self.tasks.get(self.selected).map(|t| t.name.clone());
        let prev_idx = self.selected;
        self.tasks = next_tasks;
        self.selected = next_selection(prev_idx, prev_name.as_deref(), &self.tasks);
        self.daemon_alive = crate::cache::is_daemon_alive();
        // Selected pane index might now be out of bounds.
        let pane_count = self
            .tasks
            .get(self.selected)
            .map(|t| t.panes.len())
            .unwrap_or(0);
        if self.panes_selected >= pane_count {
            self.panes_selected = pane_count.saturating_sub(1);
        }
    }

    pub fn selected_task(&self) -> Option<&TaskView> {
        self.tasks.get(self.selected)
    }
}

/// Pick the new selection index after the task list changes. Follows
/// the task by name (so reorders / inserts above don't shift the
/// cursor). If the task is gone, keep the previous index clamped to
/// the new list length.
fn next_selection(prev_idx: usize, prev_name: Option<&str>, new_tasks: &[TaskView]) -> usize {
    if let Some(name) = prev_name {
        if let Some(pos) = new_tasks.iter().position(|t| t.name == name) {
            return pos;
        }
    }
    prev_idx.min(new_tasks.len().saturating_sub(1))
}

/// Pick the task that matches the user's current working context, so
/// launching `orch tui` from inside a task worktree (or via a tmux popup
/// that inherits the surrounding pane's CWD) lands on that task. Tries
/// CWD prefix first; falls back to the user's attached tmux client's
/// current session (works inside popups since `client_session` reflects
/// the user's main view, not the popup's pane).
fn current_task_index(tasks: &[TaskView]) -> Option<usize> {
    if let Some(cwd) = std::env::current_dir().ok().and_then(|p| p.canonicalize().ok()) {
        for (i, task) in tasks.iter().enumerate() {
            let wt = state::expand_home(&task.record.worktree.path);
            if wt.is_empty() {
                continue;
            }
            if let Ok(canon) = std::path::Path::new(&wt).canonicalize() {
                if cwd.starts_with(&canon) {
                    return Some(i);
                }
            }
        }
    }
    if std::env::var("TMUX").is_ok() {
        if let Ok(out) = Command::new("tmux")
            .args(["display-message", "-p", "#{client_session}"])
            .output()
        {
            if out.status.success() {
                let session = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !session.is_empty() {
                    for (i, task) in tasks.iter().enumerate() {
                        if state::session_matches(&session, &task.record.tmux.session_name) {
                            return Some(i);
                        }
                    }
                }
            }
        }
    }
    None
}

fn status_from_str(s: &str) -> TaskStatus {
    match s {
        "ready" => TaskStatus::Ready,
        "working" => TaskStatus::Working,
        "unknown" => TaskStatus::Unknown,
        "attached" => TaskStatus::Attached,
        "paused" => TaskStatus::Paused,
        "error" => TaskStatus::Error,
        _ => TaskStatus::Idle,
    }
}

/// List tmux panes that belong to a session. Returns empty if session
/// doesn't exist or tmux isn't running.
fn panes_for_session(session: &str) -> Vec<TmuxPaneInfo> {
    if session.is_empty() {
        return Vec::new();
    }
    // tmux's session_matches handles numeric prefixes.
    let actual = match find_actual_session(session) {
        Some(s) => s,
        None => return Vec::new(),
    };
    let output = Command::new("tmux")
        .args([
            "list-panes", "-s", "-t", &actual, "-F",
            "#{pane_id}|#{session_name}|#{pane_current_command}|#{pane_active}",
        ])
        .stderr(Stdio::null())
        .output()
        .ok();
    let Some(output) = output.filter(|o| o.status.success()) else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(4, '|');
            let id = parts.next()?.to_string();
            let session = parts.next()?.to_string();
            let command = parts.next()?.to_string();
            let active = parts.next()? == "1";
            Some(TmuxPaneInfo {
                id,
                session,
                command,
                active,
            })
        })
        .collect()
}

fn find_actual_session(expected: &str) -> Option<String> {
    let output = Command::new("tmux")
        .args(["list-sessions", "-F", "#{session_name}"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find(|n| state::session_matches(n, expected))
        .map(String::from)
}

// Rendering.

pub fn render(frame: &mut Frame, app: &mut App) {
    let screen = frame.area();
    let layout = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(screen);
    let area = layout[0];
    let footer = layout[1];

    // Fullscreen take-over: when drilled into a PR, the diff gets the
    // whole terminal. Esc returns to the three-pane layout. The detail
    // tab must still be Prs — if the user pressed `1`/`3`/`4` to jump
    // to another tab while drilled, we treat that as an exit (the
    // dispatcher resets pr_view) so this short-circuit is consistent.
    if let PrView::Detail {
        number,
        focus,
        file_cursor,
        scroll,
    } = &app.pr_view
    {
        if app.detail_tab == Tab::Prs {
            render_pr_detail_fullscreen(frame, area, app, *number, *focus, *file_cursor, *scroll);
            render_shortcut_footer(frame, footer, app);
            if app.show_help {
                render_help_overlay_pr_detail(frame, screen);
            }
            if app.message_input.is_some() {
                render_message_input(frame, screen, app);
            }
            return;
        }
    }

    let outer = Layout::horizontal([
        Constraint::Length(LIST_WIDTH),
        Constraint::Length(SEPARATOR_WIDTH),
        Constraint::Min(0),
    ])
    .split(area);

    render_list(frame, outer[0], app);
    render_vertical_separator(frame, outer[1]);

    let preview_active = app.focus == Pane::Right
        && match app.detail_tab {
            Tab::Prs => pr_preview_target(app).is_some(),
            _ => false,
        };
    if app.show_activity || preview_active {
        let right = Layout::vertical([
            Constraint::Percentage(100 - LOG_HEIGHT_RATIO),
            Constraint::Length(1),
            Constraint::Min(3),
        ])
        .split(outer[2]);
        render_details(frame, right[0], app);
        render_horizontal_separator(frame, right[1]);
        render_log(frame, right[2], app);
    } else {
        render_details(frame, outer[2], app);
    }

    render_shortcut_footer(frame, footer, app);
    if app.show_help {
        render_help_overlay(frame, screen);
    }
    if app.message_input.is_some() {
        render_message_input(frame, screen, app);
    }
}

/// Message input modal anchored to the bottom of the screen.
/// Single line for short input; grows upward as the buffer wraps so
/// long messages stay visible. Always renders the keymap hint as a
/// dedicated line above the input.
fn render_message_input(frame: &mut Frame, area: Rect, app: &App) {
    let Some(buf) = app.message_input.as_ref() else {
        return;
    };
    if area.height < 2 {
        return;
    }
    let prompt = " orch ▸ ";
    let prompt_width = prompt.chars().count();
    let usable = (area.width as usize).saturating_sub(prompt_width).max(1);
    // +1 for the trailing cursor glyph.
    let content_chars = buf.chars().count() + 1;
    // Visual rows needed (ceil division).
    let input_rows = ((content_chars + usable - 1) / usable).max(1) as u16;
    let total_rows = (input_rows + 1).min(area.height); // +1 for hint line

    let bar = Rect {
        x: area.x,
        y: area.y + area.height - total_rows,
        width: area.width,
        height: total_rows,
    };
    frame.render_widget(ratatui::widgets::Clear, bar);

    // Hint line on top.
    let hint_area = Rect {
        x: bar.x,
        y: bar.y,
        width: bar.width,
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(Line::styled(
            " Enter to send · Esc to cancel",
            Style::default().fg(palette().muted),
        )),
        hint_area,
    );

    // Input area below it.
    let input_area = Rect {
        x: bar.x,
        y: bar.y + 1,
        width: bar.width,
        height: total_rows.saturating_sub(1).max(1),
    };
    let line = Line::from(vec![
        Span::styled(prompt, Style::default().fg(palette().love)),
        Span::styled(buf.clone(), Style::default().fg(palette().text)),
        Span::styled("▌", Style::default().fg(palette().love)),
    ]);
    frame.render_widget(
        Paragraph::new(line).wrap(Wrap { trim: false }),
        input_area,
    );
}

fn render_vertical_separator(frame: &mut Frame, area: Rect) {
    let mut lines = Vec::with_capacity(area.height as usize);
    for _ in 0..area.height {
        lines.push(Line::styled("│", Style::default().fg(palette().muted)));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_horizontal_separator(frame: &mut Frame, area: Rect) {
    let bar = "─".repeat(area.width as usize);
    frame.render_widget(
        Paragraph::new(Line::styled(bar, Style::default().fg(palette().muted))),
        area,
    );
}

fn render_shortcut_footer(frame: &mut Frame, area: Rect, app: &App) {
    let (text, color) = if let Some(toast) = &app.toast {
        (format!(" {toast}"), palette().gold)
    } else if matches!(app.pr_view, PrView::Detail { .. }) {
        (
            " j/k move · Tab files/diff · [/] file · o open · Esc back · ? keys"
                .to_string(),
            palette().muted,
        )
    } else {
        (shortcut_hint(app), palette().muted)
    };
    frame.render_widget(
        Paragraph::new(Line::styled(
            text,
            Style::default().fg(color),
        )),
        area,
    );
}

fn shortcut_hint(app: &App) -> String {
    let Some(task) = app.selected_task() else {
        return " ? keys · q quit".to_string();
    };

    let hint = match app.focus {
        Pane::List => {
            let mut actions = vec!["j/k move"];
            if !task.record.tmux.session_name.is_empty() {
                actions.push("Enter attach");
            }
            match task.status {
                TaskStatus::Idle => actions.push("s start"),
                TaskStatus::Paused => actions.push("R resume"),
                TaskStatus::Ready
                | TaskStatus::Working
                | TaskStatus::Unknown
                | TaskStatus::Attached
                | TaskStatus::Error => actions.push("p pause"),
            }
            actions.push("? keys");
            actions.join(" · ")
        }
        Pane::Right => {
            let mut actions = match app.detail_tab {
                Tab::Overview => Vec::new(),
                Tab::Prs if task.prs.is_empty() => Vec::new(),
                Tab::Prs => vec!["j/k move", "Enter diff", "o open"],
                Tab::Panes if task.panes.is_empty() => Vec::new(),
                Tab::Panes => vec!["j/k move", "Enter attach"],
            };
            actions.extend(["H/L tabs", "Esc tasks", "? keys"]);
            actions.join(" · ")
        }
    };
    format!(" {hint}")
}

fn render_list(frame: &mut Frame, area: Rect, app: &App) {
    let mut lines: Vec<Line> = Vec::new();
    let focused = app.focus == Pane::List;

    // Header.
    let header_style = if focused {
        Style::default().fg(palette().text)
    } else {
        Style::default().fg(palette().muted)
    };
    let position = if app.tasks.is_empty() {
        "0/0".to_string()
    } else {
        format!("{}/{}", app.selected + 1, app.tasks.len())
    };
    lines.push(Line::from(vec![
        Span::styled(" tasks", header_style),
        Span::styled(
            format!("  {position}"),
            Style::default().fg(palette().muted),
        ),
    ]));
    lines.push(Line::styled(
        "─".repeat(area.width as usize),
        Style::default().fg(palette().muted),
    ));

    if app.tasks.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::styled(
            " no tasks",
            Style::default().fg(palette().subtle),
        ));
    } else {
        let name_color = if focused { palette().text } else { palette().subtle };
        for (i, task) in app.tasks.iter().enumerate() {
            let selected = i == app.selected;
            let badge = status_str(task.status);
            let badge_color = status_color(task.status);
            // P/L counts intentionally omitted from the list — the
            // user only cared about drift state at a glance, and the
            // Detail tabs already surface PR counts.
            let mut counts = String::new();
            if task.drift() {
                counts.push_str(" ⚠");
            }
            let badge_text = format!(" {badge}");
            // Positional rank in open_order — closing a task renumbers
            // the rest visually so there's no gap. The durable
            // `task.record.id` stays unchanged for persistence.
            let id_text = format!("#{} ", i + 1);
            let cursor = if selected { "▸ " } else { "  " };

            // Width available for the name itself = total - cursor (2) -
            // id - counts - badge - trailing space (1).
            let reserved = 2
                + id_text.chars().count()
                + counts.chars().count()
                + badge_text.chars().count()
                + 1;
            let name_room = (area.width as usize).saturating_sub(reserved);
            let name_str = truncate(&task.name, name_room);
            let pad = name_room.saturating_sub(name_str.chars().count());

            let mut spans = vec![
                Span::styled(
                    cursor,
                    Style::default().fg(if selected { palette().love } else { palette().muted }),
                ),
                Span::styled(id_text, Style::default().fg(palette().muted)),
                Span::styled(name_str, Style::default().fg(name_color)),
                Span::raw(" ".repeat(pad)),
            ];
            if !counts.is_empty() {
                let counts_color = if task.drift() && counts.starts_with(" ⚠") {
                    palette().love
                } else {
                    palette().subtle
                };
                spans.push(Span::styled(counts, Style::default().fg(counts_color)));
            }
            spans.push(Span::styled(
                badge_text,
                Style::default().fg(badge_color),
            ));

            let mut line = Line::from(spans);
            if selected {
                line = line.style(Style::default().bg(palette().highlight_low));
            }
            lines.push(line);
        }
    }

    frame.render_widget(Paragraph::new(lines), area);
}

fn render_details(frame: &mut Frame, area: Rect, app: &mut App) {
    if app.tasks.is_empty() {
        let placeholder = Paragraph::new(vec![
            Line::raw(""),
            Line::styled(" select a task", Style::default().fg(palette().subtle)),
        ]);
        frame.render_widget(placeholder, area);
        return;
    }

    let task = match app.selected_task() {
        Some(t) => t.clone(),
        None => return,
    };

    // Tab bar.
    let focused = app.focus == Pane::Right;
    let tabs = [Tab::Overview, Tab::Prs, Tab::Panes];
    let mut tab_spans: Vec<Span> = Vec::new();
    tab_spans.push(Span::raw(" "));
    for (i, tab) in tabs.iter().enumerate() {
        let active = *tab == app.detail_tab;
        let style = if active {
            Style::default().fg(if focused { palette().love } else { palette().text })
        } else if focused {
            Style::default().fg(palette().subtle)
        } else {
            Style::default().fg(palette().muted)
        };
        let count = match tab {
            Tab::Overview => 0,
            Tab::Prs => task.prs.len(),
            Tab::Panes => task.panes.len(),
        };
        let label = if count == 0 {
            tab.label().to_string()
        } else {
            format!("{} {count}", tab.label())
        };
        tab_spans.push(Span::styled(label, style));
        if i + 1 < tabs.len() {
            tab_spans.push(Span::styled("  ·  ", Style::default().fg(palette().muted)));
        }
    }
    let tab_bar = Paragraph::new(Line::from(tab_spans));
    let tab_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: 1,
    };
    frame.render_widget(tab_bar, tab_area);
    let divider = Paragraph::new(Line::styled(
        "─".repeat(area.width as usize),
        Style::default().fg(palette().muted),
    ));
    let div_area = Rect {
        x: area.x,
        y: area.y + 1,
        width: area.width,
        height: 1,
    };
    frame.render_widget(divider, div_area);

    let body_area = Rect {
        x: area.x,
        y: area.y + TAB_BAR_HEIGHT,
        width: area.width,
        height: area.height.saturating_sub(TAB_BAR_HEIGHT),
    };

    match app.detail_tab {
        Tab::Overview => render_tab_overview(frame, body_area, app, &task),
        Tab::Prs => render_tab_prs(frame, body_area, app, &task),
        Tab::Panes => render_tab_panes(frame, body_area, app, &task),
    }
}

fn render_tab_overview(frame: &mut Frame, area: Rect, _app: &App, task: &TaskView) {
    let session_str = if task.record.tmux.session_name.is_empty() {
        "—".to_string()
    } else {
        task.record.tmux.session_name.clone()
    };
    let worktree_str = if task.record.worktree.path.is_empty() {
        "—".to_string()
    } else {
        display_worktree_path(&task.record.worktree.path)
    };
    let prs_str = task.prs.len().to_string();
    let panes_str = task.panes.len().to_string();

    let mut lines = vec![
        Line::raw(""),
        Line::styled(
            format!(" {}", task.name),
            Style::default().fg(palette().text),
        ),
        Line::from(vec![
            Span::styled(
                format!(" {}", status_str(task.status)),
                Style::default().fg(status_color(task.status)),
            ),
            Span::styled(
                format!("  ·  {panes_str} panes"),
                Style::default().fg(palette().subtle),
            ),
        ]),
        Line::raw(""),
        kv_line(" session   ", &session_str),
        kv_line(" worktree  ", &worktree_str),
    ];
    if !task.prs.is_empty() {
        lines.push(kv_line(" prs       ", &prs_str));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn display_worktree_path(path: &str) -> String {
    let repo = std::env::var("ORCH_REPO").ok();
    display_worktree_path_with_repo(path, repo.as_deref())
}

fn display_worktree_path_with_repo(path: &str, repo: Option<&str>) -> String {
    let expanded = state::expand_home(path);
    let worktree = std::path::Path::new(&expanded);

    if let Some(repo) = repo {
        let repo = state::expand_home(repo);
        if let Ok(rel) = worktree.strip_prefix(std::path::Path::new(&repo)) {
            let rel = rel.to_string_lossy();
            if !rel.is_empty() {
                return rel.to_string();
            }
        }
    }

    worktree
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string())
}

fn render_tab_prs(frame: &mut Frame, area: Rect, app: &App, task: &TaskView) {
    let focused = app.focus == Pane::Right && app.detail_tab == Tab::Prs;
    let cursor_number = match &app.pr_view {
        PrView::List { cursor_number } => *cursor_number,
        PrView::Detail { number, .. } => *number,
    };

    let mut lines = vec![Line::raw("")];
    if task.prs.is_empty() {
        lines.push(Line::styled(
            " (no linked PRs)",
            Style::default().fg(palette().subtle),
        ));
        lines.push(Line::styled(
            " orch pr add <task> <number>",
            Style::default().fg(palette().muted),
        ));
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
        return;
    }

    let width = area.width as usize;
    for pr in &task.prs {
        let selected = focused && pr.number == cursor_number;
        let cursor_glyph = if selected { " ▸ " } else { "   " };
        let cursor_color = if selected { palette().love } else { palette().muted };
        let id_color = if selected { palette().love } else { palette().iris };

        let title = if pr.title.is_empty() {
            "(no title cached)".into()
        } else {
            pr.title.clone()
        };

        // State badge right-aligned so a wrapped branch name doesn't merge with it.
        let state_badge: Option<(&str, ratatui::style::Color)> = match pr.state.as_str() {
            "MERGED" => Some(("merged", palette().iris)),
            "CLOSED" => Some(("closed", palette().muted)),
            _ => None,
        };
        let id_text = format!("#{}", pr.number);
        let badge_text = state_badge.map(|(t, _)| t).unwrap_or("");
        // 3 (cursor) + id + 2 (sep) + title room + 1 (sep) + badge + 1 (pad)
        let reserved = 3
            + id_text.chars().count()
            + 2
            + (if badge_text.is_empty() { 0 } else { badge_text.chars().count() + 1 });
        let title_room = width.saturating_sub(reserved);
        let title_str = truncate(&title, title_room);
        let pad = title_room.saturating_sub(title_str.chars().count());
        let mut row1 = vec![
            Span::styled(cursor_glyph, Style::default().fg(cursor_color)),
            Span::styled(id_text, Style::default().fg(id_color)),
            Span::raw("  "),
            Span::styled(title_str, Style::default().fg(palette().text)),
            Span::raw(" ".repeat(pad)),
        ];
        if let Some((t, color)) = state_badge {
            row1.push(Span::styled(t.to_string(), Style::default().fg(color)));
        }
        lines.push(Line::from(row1));

        // Row 2: meta strip — ci · review · codex · age · branch (truncated).
        let mut meta: Vec<Span> = vec![Span::raw("    ")];
        meta.push(match pr.ci_pass {
            Some(true) => Span::styled("✓ ci", Style::default().fg(palette().pine)),
            Some(false) => Span::styled("✗ ci", Style::default().fg(palette().love)),
            None => Span::styled("· ci", Style::default().fg(palette().muted)),
        });
        meta.push(if pr.approved {
            Span::styled("  ·  ✓ review", Style::default().fg(palette().pine))
        } else {
            Span::styled("  ·  · review", Style::default().fg(palette().muted))
        });
        meta.push(match pr.codex {
            crate::state::CodexStatus::ThumbsUp => {
                Span::styled("  ·  ✓ codex", Style::default().fg(palette().pine))
            }
            crate::state::CodexStatus::Commented => Span::styled(
                "  ·  · codex commented",
                Style::default().fg(palette().gold),
            ),
            crate::state::CodexStatus::None => {
                Span::styled("  ·  · codex", Style::default().fg(palette().muted))
            }
        });
        let age = relative_age(&pr.updated_at);
        if !age.is_empty() {
            meta.push(Span::styled(
                format!("  ·  {age}"),
                Style::default().fg(palette().muted),
            ));
        }
        if !pr.head_branch.is_empty() {
            // Branch can be long ("ashley/ENG-29187-scrub-…"). Truncate
            // so the row fits on one terminal line.
            let branch_room = 36;
            let branch = truncate(&pr.head_branch, branch_room);
            meta.push(Span::styled(
                format!("  ·  {branch}"),
                Style::default().fg(palette().muted),
            ));
        }
        // Mergeable: glyph only on conflict (skip cell noise on green path).
        if pr.mergeable.as_deref() == Some("CONFLICTING") {
            meta.push(Span::styled(
                "  ⚠ conflict",
                Style::default().fg(palette().gold),
            ));
        }
        lines.push(Line::from(meta));

        // Row 3: stats — only when we have churn data.
        if pr.changed_files > 0 || pr.additions > 0 || pr.deletions > 0 {
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(
                    format!("+{} / -{}", pr.additions, pr.deletions),
                    Style::default().fg(palette().subtle),
                ),
                Span::styled(
                    format!("  ·  {} files", pr.changed_files),
                    Style::default().fg(palette().muted),
                ),
            ]));
        }
        lines.push(Line::raw(""));
    }
    if focused {
        lines.push(Line::styled(
            " j/k move · Enter open · o browser",
            Style::default().fg(palette().muted),
        ));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

/// Full-screen `render_pr_detail`: 40-col file list + toast overlay
/// (since `render_log` doesn't run here).
fn render_pr_detail_fullscreen(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    number: u32,
    focus: PrDetailFocus,
    file_cursor: usize,
    scroll: u16,
) {
    // Defensive clamp — refresh might have produced fewer files.
    let cache = crate::cache::read_pr_diffs();
    let n_files = cache.diffs.get(&number).map(|d| d.files.len()).unwrap_or(0);
    let safe_cursor = if n_files == 0 { 0 } else { file_cursor.min(n_files - 1) };

    // Reserve one row for the toast overlay if present.
    let toast_row: u16 = if app.toast.is_some() { 1 } else { 0 };
    let detail_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: area.height.saturating_sub(toast_row),
    };

    render_pr_detail_with_widths(
        frame,
        detail_area,
        true,
        number,
        focus,
        safe_cursor,
        scroll,
        40,
    );

    if let Some(t) = &app.toast {
        let toast_y = area.y + area.height.saturating_sub(1);
        frame.render_widget(
            Paragraph::new(Line::styled(
                format!(" {t}"),
                Style::default().fg(palette().gold),
            )),
            Rect { x: area.x, y: toast_y, width: area.width, height: 1 },
        );
    }
}

/// PR detail view: file list left, diff right.
/// Tab toggles `PrDetailFocus::Files ↔ Diff`.
///
/// Layout (no horizontal rules — content-forward, alignment does the work):
/// ```text
/// row 0   #N  title                                                   merged
/// row 1   ✓ ci · ✓ review · · codex · 3d ago · branch  ·  +A/-D  ·  N files
/// row 2   <blank>
/// row 3+  body — file list left | diff body right
/// row -1  footer hint (focused only)
/// ```
fn render_pr_detail_with_widths(
    frame: &mut Frame,
    area: Rect,
    focused: bool,
    number: u32,
    focus: PrDetailFocus,
    file_cursor: usize,
    scroll: u16,
    file_col_width: u16,
) {
    let pr_cache = crate::cache::read_prs();
    let diff_cache = crate::cache::read_pr_diffs();
    let pr = pr_cache.prs.get(&number);
    let diff = diff_cache.diffs.get(&number);

    let footer_height: u16 = if focused { 1 } else { 0 };

    // Row 0 — title with right-aligned state badge.
    let title = pr.map(|p| p.title.clone()).unwrap_or_default();
    let state_badge: Option<(&str, ratatui::style::Color)> =
        pr.and_then(|p| match p.state.as_str() {
            "MERGED" => Some(("merged", palette().iris)),
            "CLOSED" => Some(("closed", palette().muted)),
            _ => None,
        });
    let id_text = format!(" #{number}");
    let badge_text = state_badge.map(|(t, _)| t).unwrap_or("");
    let reserved = id_text.chars().count()
        + 2
        + (if badge_text.is_empty() { 0 } else { badge_text.chars().count() + 1 });
    let title_room = (area.width as usize).saturating_sub(reserved);
    let title_str = truncate(&title, title_room);
    let title_pad = title_room.saturating_sub(title_str.chars().count());
    let mut row0 = vec![
        Span::styled(id_text, Style::default().fg(palette().iris)),
        Span::raw("  "),
        Span::styled(title_str, Style::default().fg(palette().text)),
        Span::raw(" ".repeat(title_pad)),
    ];
    if let Some((t, color)) = state_badge {
        row0.push(Span::styled(t.to_string(), Style::default().fg(color)));
    }
    frame.render_widget(
        Paragraph::new(Line::from(row0)),
        Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 1,
        },
    );

    // Row 1 — meta strip. Single cadence: `  ·  ` between groups.
    let mut meta: Vec<Span> = vec![Span::raw(" ")];
    if let Some(c) = pr {
        meta.push(match c.ci_pass {
            Some(true) => Span::styled("✓ ci", Style::default().fg(palette().pine)),
            Some(false) => Span::styled("✗ ci", Style::default().fg(palette().love)),
            None => Span::styled("· ci", Style::default().fg(palette().muted)),
        });
        meta.push(if c.approved {
            Span::styled("  ·  ✓ review", Style::default().fg(palette().pine))
        } else {
            Span::styled("  ·  · review", Style::default().fg(palette().muted))
        });
        meta.push(match c.codex.as_str() {
            "ThumbsUp" => Span::styled("  ·  ✓ codex", Style::default().fg(palette().pine)),
            "Commented" => Span::styled(
                "  ·  · codex commented",
                Style::default().fg(palette().gold),
            ),
            _ => Span::styled("  ·  · codex", Style::default().fg(palette().muted)),
        });
        let age = relative_age(&c.updated_at);
        if !age.is_empty() {
            meta.push(Span::styled(
                format!("  ·  {age}"),
                Style::default().fg(palette().muted),
            ));
        }
        if !c.head_branch.is_empty() {
            meta.push(Span::styled(
                format!("  ·  {} → main", truncate(&c.head_branch, 36)),
                Style::default().fg(palette().muted),
            ));
        }
        meta.push(Span::styled(
            format!("  ·  +{} / -{}", c.additions, c.deletions),
            Style::default().fg(palette().subtle),
        ));
        meta.push(Span::styled(
            format!("  ·  {} files", c.changed_files),
            Style::default().fg(palette().muted),
        ));
        if c.mergeable.as_deref() == Some("CONFLICTING") {
            meta.push(Span::styled(
                "  ·  ⚠ conflict",
                Style::default().fg(palette().gold),
            ));
        }
    }
    frame.render_widget(
        Paragraph::new(Line::from(meta)),
        Rect {
            x: area.x,
            y: area.y + 1,
            width: area.width,
            height: 1,
        },
    );

    // Row 2 — blank gap.
    let mut body_top: u16 = 3;

    // Optional row 3 — stale-diff banner.
    let stale = match (pr, diff) {
        (Some(p), Some(d))
            if !p.head_sha.is_empty() && !d.head_sha.is_empty() && p.head_sha != d.head_sha =>
        {
            true
        }
        _ => false,
    };
    if stale {
        frame.render_widget(
            Paragraph::new(Line::styled(
                " diff stale (head moved) · r refresh",
                Style::default().fg(palette().gold),
            )),
            Rect {
                x: area.x,
                y: area.y + body_top,
                width: area.width,
                height: 1,
            },
        );
        body_top = body_top.saturating_add(2); // banner + 1 blank
    }

    let body_area = Rect {
        x: area.x,
        y: area.y + body_top,
        width: area.width,
        height: area.height
            .saturating_sub(body_top)
            .saturating_sub(footer_height),
    };

    // Footer.
    if focused {
        let hint = match focus {
            PrDetailFocus::Files => {
                " j/k file · Tab diff · ]/[ next/prev · r refresh · o browser · Esc back"
            }
            PrDetailFocus::Diff => {
                " j/k scroll · H/L hunk · Tab files · ]/[ next/prev · r refresh · o browser · Esc back"
            }
        };
        frame.render_widget(
            Paragraph::new(Line::styled(hint, Style::default().fg(palette().muted))),
            Rect {
                x: area.x,
                y: area.y + area.height.saturating_sub(1),
                width: area.width,
                height: 1,
            },
        );
    }

    // Body — five states.
    let Some(d) = diff else {
        let mut lines = vec![Line::raw("")];
        if pr.map(|p| p.head_sha.is_empty()).unwrap_or(true) {
            lines.push(Line::styled(
                " PR metadata not yet fetched.",
                Style::default().fg(palette().muted),
            ));
            lines.push(Line::styled(
                " Wait for the next PR loop cycle (~30s) or restart `orch daemon`.",
                Style::default().fg(palette().subtle),
            ));
        } else {
            lines.push(Line::styled(
                " diff loading…",
                Style::default().fg(palette().muted),
            ));
            lines.push(Line::styled(
                " (refreshing in the background; press r to retry)",
                Style::default().fg(palette().subtle),
            ));
        }
        frame.render_widget(Paragraph::new(lines), body_area);
        return;
    };

    if let Some(err) = &d.error {
        let mut lines = vec![Line::raw("")];
        lines.push(Line::styled(
            format!(" diff fetch failed: {err}"),
            Style::default().fg(palette().love),
        ));
        lines.push(Line::styled(
            " r retry · o browser · Esc back",
            Style::default().fg(palette().muted),
        ));
        frame.render_widget(Paragraph::new(lines), body_area);
        return;
    }

    if d.truncated {
        let mut lines = vec![Line::raw("")];
        lines.push(Line::styled(
            format!(
                " diff is {:.1} MB · too large to render",
                (d.raw_size as f64) / 1_000_000.0,
            ),
            Style::default().fg(palette().gold),
        ));
        lines.push(Line::styled(
            " press o to open in browser",
            Style::default().fg(palette().muted),
        ));
        frame.render_widget(Paragraph::new(lines), body_area);
        return;
    }

    if d.files.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::styled(
                " no changes",
                Style::default().fg(palette().muted),
            )),
            body_area,
        );
        return;
    }

    // Two-pane split: file list (left) | diff body (right).
    let file_col_width: u16 = file_col_width.min(area.width / 2);
    let file_area = Rect {
        x: body_area.x,
        y: body_area.y,
        width: file_col_width,
        height: body_area.height,
    };
    let diff_area = Rect {
        x: body_area.x + file_col_width,
        y: body_area.y,
        width: body_area.width.saturating_sub(file_col_width),
        height: body_area.height,
    };

    render_pr_file_list(frame, file_area, &d.files, file_cursor, focus, focused);
    render_pr_diff_body(frame, diff_area, &d.files, file_cursor, scroll);
}

fn render_pr_file_list(
    frame: &mut Frame,
    area: Rect,
    files: &[crate::cache::CachedPrDiffFile],
    cursor: usize,
    focus: PrDetailFocus,
    focused: bool,
) {
    if files.is_empty() {
        frame.render_widget(Paragraph::new(Vec::<Line>::new()), area);
        return;
    }

    // Strip the common path prefix shared by all files so the row shows
    // only what differentiates. Prefix gets its own dim header line; the
    // user always knows the full path at a glance.
    let raw_paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    let prefix = longest_common_path_prefix(&raw_paths);
    let stripped: Vec<&str> = raw_paths
        .iter()
        .map(|p| p.strip_prefix(prefix.as_str()).unwrap_or(p))
        .collect();

    let mut lines: Vec<Line> = Vec::new();
    if !prefix.is_empty() {
        let label = if prefix.chars().count() > area.width as usize {
            truncate_tail(&prefix, area.width as usize)
        } else {
            prefix.clone()
        };
        lines.push(Line::styled(label, Style::default().fg(palette().muted)));
    }
    let header_rows = lines.len();
    let visible = (area.height as usize).saturating_sub(header_rows);
    if visible == 0 {
        frame.render_widget(Paragraph::new(lines), area);
        return;
    }

    let start = cursor.saturating_sub(visible.saturating_sub(2)).min(
        files.len().saturating_sub(visible.min(files.len())),
    );
    for (i, f) in files.iter().enumerate().skip(start).take(visible) {
        let is_cur = i == cursor;
        let cur_focused = is_cur && focused && matches!(focus, PrDetailFocus::Files);
        let glyph = if is_cur { "▸ " } else { "  " };
        let glyph_color = if cur_focused { palette().love } else { palette().muted };
        let path_color = if is_cur { palette().text } else { palette().subtle };
        let stats = format!("+{}/-{}", f.additions, f.deletions);
        // path + stats fits in `area.width` minus glyph (2) + " " + stats.
        let stats_room = stats.chars().count();
        let path_room = (area.width as usize).saturating_sub(2 + 1 + stats_room);
        let path = truncate_tail(stripped[i], path_room);
        let pad = path_room.saturating_sub(path.chars().count());

        let base_style = if is_cur && focused {
            Style::default().bg(palette().highlight_low)
        } else {
            Style::default()
        };

        let mut spans = vec![
            Span::styled(
                glyph,
                base_style.fg(glyph_color),
            ),
            Span::styled(
                path,
                base_style.fg(path_color),
            ),
            Span::styled(
                " ".repeat(pad),
                base_style,
            ),
            Span::styled(" ", base_style),
            Span::styled(stats, base_style.fg(palette().muted)),
        ];
        // Pad the highlight bar across the full workspace width so it
        // reads as a continuous row (not a fragment around the text).
        if is_cur && focused {
            let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
            let extra = (area.width as usize).saturating_sub(used);
            if extra > 0 {
                spans.push(Span::styled(" ".repeat(extra), base_style));
            }
        }
        lines.push(Line::from(spans));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

/// Longest common path prefix across `paths`, split on `/` boundaries
/// (so we never strip half a directory name). Always retains the
/// trailing `/` when non-empty. Stops one segment short of the shortest
/// path so the basename is preserved.
fn longest_common_path_prefix(paths: &[&str]) -> String {
    if paths.is_empty() {
        return String::new();
    }
    let segs: Vec<Vec<&str>> = paths.iter().map(|p| p.split('/').collect()).collect();
    let min_len = segs.iter().map(|s| s.len()).min().unwrap_or(0);
    let mut prefix = String::new();
    for i in 0..min_len.saturating_sub(1) {
        let first = segs[0][i];
        if segs.iter().all(|s| s[i] == first) {
            prefix.push_str(first);
            prefix.push('/');
        } else {
            break;
        }
    }
    prefix
}

fn render_pr_diff_body(
    frame: &mut Frame,
    area: Rect,
    files: &[crate::cache::CachedPrDiffFile],
    file_cursor: usize,
    scroll: u16,
) {
    let Some(file) = files.get(file_cursor) else {
        return;
    };
    let (lines, _) = build_pr_diff_lines(file, area.width);

    // Each line is one terminal row (we truncate, not wrap, to keep
    // long SQL/JSON readable). Scroll = simple line offset.
    let total = lines.len() as u16;
    let max_scroll = total.saturating_sub(area.height);
    let scroll = scroll.min(max_scroll);

    frame.render_widget(
        Paragraph::new(lines).scroll((scroll, 0)),
        area,
    );
}

/// Build the rendered diff body for a file. Returns `(lines, hunk_anchor_rows)`
/// where `hunk_anchor_rows[i]` is the line index of hunk i's header within
/// `lines`. Lines longer than `body_width` are truncated with `…` so each
/// diff line maps to exactly one terminal row — wrapping 200-char SQL
/// across 3 rows is unreadable in practice.
fn build_pr_diff_lines(
    file: &crate::cache::CachedPrDiffFile,
    body_width: u16,
) -> (Vec<Line<'static>>, Vec<u16>) {
    let width = body_width as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut hunk_anchors: Vec<u16> = Vec::new();

    lines.push(Line::from(vec![
        Span::raw(" "),
        Span::styled(file.path.clone(), Style::default().fg(palette().text)),
    ]));
    if file.status != "modified" {
        lines.push(Line::styled(
            format!(" ({})", file.status),
            Style::default().fg(palette().muted),
        ));
    }
    lines.push(Line::raw(""));

    if file.status == "binary" {
        lines.push(Line::styled(
            " binary file · diff suppressed",
            Style::default().fg(palette().muted),
        ));
        return (lines, hunk_anchors);
    }

    for hunk in &file.hunks {
        hunk_anchors.push(lines.len() as u16);
        if let Some((header_part, ctx)) = split_hunk_header(&hunk.header) {
            let mut spans = vec![
                Span::raw(" "),
                Span::styled(header_part.to_string(), Style::default().fg(palette().muted)),
            ];
            if !ctx.is_empty() {
                spans.push(Span::styled(
                    format!(" {ctx}"),
                    Style::default().fg(palette().subtle),
                ));
            }
            lines.push(Line::from(spans));
        } else {
            lines.push(Line::styled(
                format!(" {}", hunk.header),
                Style::default().fg(palette().muted),
            ));
        }
        // Body cells available for the line content after the ` X ` glyph.
        let body_room = width.saturating_sub(3);
        // Tab width — 4 spaces. Source lines (Go/Rust/etc) use \t for
        // indentation; the terminal's default tab stop (often 8) breaks
        // our char-count truncation and bg-pad math.
        const TAB_WIDTH: usize = 4;
        let tab_replacement = " ".repeat(TAB_WIDTH);
        for line in &hunk.lines {
            // Color the glyph by status while keeping the body neutral.
            let (prefix, rest, glyph_color, body_color, bg) =
                if let Some(rest) = line.strip_prefix('+') {
                    ("+", rest, palette().pine, palette().text, Some(palette().diff_add))
                } else if let Some(rest) = line.strip_prefix('-') {
                    ("-", rest, palette().love, palette().text, Some(palette().diff_del))
                } else if let Some(rest) = line.strip_prefix(' ') {
                    (" ", rest, palette().muted, palette().subtle, None)
                } else {
                    (" ", line.as_str(), palette().muted, palette().muted, None)
                };
            // Expand tabs first so truncation/padding math is in cells.
            let expanded = rest.replace('\t', &tab_replacement);
            // Truncate to one terminal row; pad to the row width when
            // tinted so the bg color extends across the visible row.
            let mut visible = if expanded.chars().count() > body_room {
                truncate(&expanded, body_room)
            } else {
                expanded
            };
            if bg.is_some() {
                let pad = body_room.saturating_sub(visible.chars().count());
                if pad > 0 {
                    visible.push_str(&" ".repeat(pad));
                }
            }
            let glyph_style = match bg {
                Some(bg) => Style::default().fg(glyph_color).bg(bg),
                None => Style::default().fg(glyph_color),
            };
            let body_style = match bg {
                Some(bg) => Style::default().fg(body_color).bg(bg),
                None => Style::default().fg(body_color),
            };
            lines.push(Line::from(vec![
                Span::styled(format!(" {prefix} "), glyph_style),
                Span::styled(visible, body_style),
            ]));
        }
        lines.push(Line::raw(""));
    }

    (lines, hunk_anchors)
}

/// Split `@@ -a,b +c,d @@ context` into ("@@ -a,b +c,d @@", "context").
fn split_hunk_header(header: &str) -> Option<(&str, &str)> {
    // Find the SECOND "@@".
    let first = header.find("@@")?;
    let after_first = &header[first + 2..];
    let second_rel = after_first.find("@@")?;
    let header_end = first + 2 + second_rel + 2;
    let head = &header[..header_end];
    let ctx = header[header_end..].trim();
    Some((head, ctx))
}

/// Truncate from the LEFT — keeps the basename / file tail visible when
/// a long path overflows the workspace.
fn truncate_tail(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_string();
    }
    let take = max.saturating_sub(1);
    let from = chars.len() - take;
    let mut out = String::from("…");
    out.extend(chars[from..].iter());
    out
}

/// PR preview target — when focused on the PRs tab with a cursor on a
/// linked PR, the log pane renders the PR preview instead of the run log.
fn pr_preview_target(app: &App) -> Option<u32> {
    if app.focus != Pane::Right || app.detail_tab != Tab::Prs {
        return None;
    }
    match &app.pr_view {
        PrView::List { cursor_number } if *cursor_number > 0 => Some(*cursor_number),
        _ => None,
    }
}

/// Render a PR preview into the log-pane area: title, meta strip, churn
/// stats and top files by churn. Content-forward, with no
/// dividers.
fn render_pr_preview(frame: &mut Frame, area: Rect, number: u32) {
    let pr_cache = crate::cache::read_prs();
    let diff_cache = crate::cache::read_pr_diffs();
    let cached = pr_cache.prs.get(&number);
    let diff = diff_cache.diffs.get(&number);

    let header = format!(" preview: #{number}");
    frame.render_widget(
        Paragraph::new(Line::styled(header, Style::default().fg(palette().muted))),
        Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 1,
        },
    );

    let body_area = Rect {
        x: area.x,
        y: area.y + 1,
        width: area.width,
        height: area.height.saturating_sub(1),
    };

    let mut lines: Vec<Line> = Vec::new();

    let Some(c) = cached else {
        lines.push(Line::styled(" loading…", Style::default().fg(palette().muted)));
        frame.render_widget(Paragraph::new(lines), body_area);
        return;
    };

    // Title.
    if !c.title.is_empty() {
        lines.push(Line::from(vec![
            Span::raw(" "),
            Span::styled(c.title.clone(), Style::default().fg(palette().text)),
        ]));
    }

    // Meta strip.
    let mut meta: Vec<Span> = vec![Span::raw(" ")];
    meta.push(Span::styled(
        format!("#{}", c.number),
        Style::default().fg(palette().iris),
    ));
    meta.push(Span::raw("  "));
    meta.push(match c.ci_pass {
        Some(true) => Span::styled("✓ ci", Style::default().fg(palette().pine)),
        Some(false) => Span::styled("✗ ci", Style::default().fg(palette().love)),
        None => Span::styled("· ci", Style::default().fg(palette().muted)),
    });
    meta.push(if c.approved {
        Span::styled("  ·  ✓ review", Style::default().fg(palette().pine))
    } else {
        Span::styled("  ·  · review", Style::default().fg(palette().muted))
    });
    meta.push(match c.codex.as_str() {
        "ThumbsUp" => Span::styled("  ·  ✓ codex", Style::default().fg(palette().pine)),
        "Commented" => Span::styled("  ·  · codex commented", Style::default().fg(palette().gold)),
        _ => Span::styled("  ·  · codex", Style::default().fg(palette().muted)),
    });
    let age = relative_age(&c.updated_at);
    if !age.is_empty() {
        meta.push(Span::styled(
            format!("  ·  {age}"),
            Style::default().fg(palette().muted),
        ));
    }
    if !c.head_branch.is_empty() {
        meta.push(Span::styled(
            format!("  ·  {}", truncate(&c.head_branch, 36)),
            Style::default().fg(palette().muted),
        ));
    }
    lines.push(Line::from(meta));

    // Stats row.
    let mut stats: Vec<Span> = vec![Span::raw(" ")];
    stats.push(Span::styled(
        format!("+{} / -{}", c.additions, c.deletions),
        Style::default().fg(palette().subtle),
    ));
    stats.push(Span::styled(
        format!("  ·  {} files", c.changed_files),
        Style::default().fg(palette().muted),
    ));
    let merge_glyph = match c.mergeable.as_deref() {
        Some("CONFLICTING") => Some(("  ·  ⚠ conflict", palette().gold)),
        Some("MERGEABLE") => None,
        _ => None,
    };
    if let Some((s, color)) = merge_glyph {
        stats.push(Span::styled(s, Style::default().fg(color)));
    }
    match c.state.as_str() {
        "MERGED" => stats.push(Span::styled("  ·  merged", Style::default().fg(palette().iris))),
        "CLOSED" => stats.push(Span::styled("  ·  closed", Style::default().fg(palette().muted))),
        _ => {}
    }
    lines.push(Line::from(stats));

    // Description body — wrapped, truncated to fit. Skipped when empty.
    if !c.body.is_empty() {
        lines.push(Line::raw(""));
        let width = (body_area.width.saturating_sub(2) as usize).max(20);
        let wrapped = wrap_text(&c.body, width);
        let body_room = (body_area.height as usize).saturating_sub(lines.len());
        let take = if wrapped.len() > body_room {
            body_room.saturating_sub(1)
        } else {
            body_room
        };
        for w in wrapped.iter().take(take) {
            lines.push(Line::from(vec![
                Span::raw(" "),
                Span::styled(w.clone(), Style::default().fg(palette().subtle)),
            ]));
        }
        if wrapped.len() > take && take > 0 {
            lines.push(Line::styled(" …", Style::default().fg(palette().muted)));
        }
    }

    // Top files by churn — only when diff cache is populated AND there's
    // still room (description gets priority since it explains the change).
    if let Some(d) = diff {
        let body_room = (body_area.height as usize).saturating_sub(lines.len());
        if !d.files.is_empty() && body_room > 1 {
            lines.push(Line::raw(""));
            let mut by_churn: Vec<&crate::cache::CachedPrDiffFile> =
                d.files.iter().collect();
            by_churn.sort_by_key(|f| std::cmp::Reverse(f.additions + f.deletions));
            let body_room = (body_area.height as usize).saturating_sub(lines.len());
            let take = body_room.saturating_sub(1).min(by_churn.len());
            for f in by_churn.iter().take(take) {
                lines.push(Line::from(vec![
                    Span::raw(" "),
                    Span::styled(
                        truncate(&f.path, 32),
                        Style::default().fg(palette().subtle),
                    ),
                    Span::styled(
                        format!("  +{}/-{}", f.additions, f.deletions),
                        Style::default().fg(palette().muted),
                    ),
                ]));
            }
            let extra = by_churn.len().saturating_sub(take);
            if extra > 0 {
                lines.push(Line::styled(
                    format!(" ({extra} more)"),
                    Style::default().fg(palette().muted),
                ));
            }
        }
    }

    frame.render_widget(Paragraph::new(lines), body_area);
}

/// Wrap a text into lines fitting within `width`. Preserves blank lines.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut current = String::new();
        for word in line.split_whitespace() {
            if current.is_empty() {
                current.push_str(word);
            } else if current.chars().count() + 1 + word.chars().count() <= width {
                current.push(' ');
                current.push_str(word);
            } else {
                out.push(std::mem::take(&mut current));
                current.push_str(word);
            }
        }
        if !current.is_empty() {
            out.push(current);
        }
    }
    out
}

fn relative_age(iso: &str) -> String {
    if iso.is_empty() {
        return String::new();
    }
    let then = match parse_iso8601(iso) {
        Some(t) => t,
        None => return String::new(),
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let secs = now.saturating_sub(then);
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}

/// Naive ISO-8601 parse — only the YYYY-MM-DDTHH:MM:SS prefix matters.
fn parse_iso8601(s: &str) -> Option<u64> {
    if s.len() < 19 {
        return None;
    }
    let bytes = s.as_bytes();
    let year: u64 = std::str::from_utf8(&bytes[0..4]).ok()?.parse().ok()?;
    let month: u64 = std::str::from_utf8(&bytes[5..7]).ok()?.parse().ok()?;
    let day: u64 = std::str::from_utf8(&bytes[8..10]).ok()?.parse().ok()?;
    let hour: u64 = std::str::from_utf8(&bytes[11..13]).ok()?.parse().ok()?;
    let minute: u64 = std::str::from_utf8(&bytes[14..16]).ok()?.parse().ok()?;
    let second: u64 = std::str::from_utf8(&bytes[17..19]).ok()?.parse().ok()?;
    Some(days_since_epoch(year, month, day) * 86400 + hour * 3600 + minute * 60 + second)
}

/// Days since Unix epoch for given Y/M/D (Gregorian, naive).
fn days_since_epoch(year: u64, month: u64, day: u64) -> u64 {
    let mut days: u64 = 0;
    for y in 1970..year {
        days += if is_leap(y) { 366 } else { 365 };
    }
    let dim = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    for m in 1..month {
        days += dim[(m - 1) as usize] as u64;
        if m == 2 && is_leap(year) {
            days += 1;
        }
    }
    days + day.saturating_sub(1)
}

fn is_leap(y: u64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn render_tab_panes(frame: &mut Frame, area: Rect, app: &App, task: &TaskView) {
    let mut lines = vec![Line::raw("")];
    if task.panes.is_empty() {
        lines.push(Line::styled(
            " (no live tmux panes — task not spawned)",
            Style::default().fg(palette().subtle),
        ));
    } else {
        let focused =
            app.focus == Pane::Right && app.detail_tab == Tab::Panes;
        for (i, pane) in task.panes.iter().enumerate() {
            let selected = i == app.panes_selected;
            let marker = if pane.active { "●" } else { "·" };
            let prefix = if selected && focused { "▸" } else { " " };
            let style = if selected && focused {
                Style::default().fg(palette().love).bg(palette().highlight_low)
            } else if selected {
                Style::default().fg(palette().text).bg(palette().highlight_low)
            } else if focused {
                Style::default().fg(palette().text)
            } else {
                Style::default().fg(palette().subtle)
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!(" {prefix} {marker} "),
                    Style::default().fg(if pane.active { palette().pine } else { palette().muted }),
                ),
                Span::styled(
                    format!("{}  {}", pane.id, pane.command),
                    style,
                ),
            ]));
        }
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            " j/k navigate · Enter attach",
            Style::default().fg(palette().muted),
        ));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn render_log(frame: &mut Frame, area: Rect, app: &App) {
    if let Some(number) = pr_preview_target(app) {
        render_pr_preview(frame, area, number);
        return;
    }

    // Log is a passive viewer with no focus state.
    // Toast (if present) overrides the run-id label.
    let header_style = Style::default().fg(palette().muted);
    let header_text = if let Some(toast) = &app.toast {
        format!(" activity  {toast}")
    } else {
        match &app.log.run_id {
            Some(id) => format!(
                " activity  {}{}",
                id,
                if app.log.finished { " · done" } else { "" },
            ),
            None => " activity  none".to_string(),
        }
    };
    let header_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(Line::styled(header_text, header_style)),
        header_area,
    );

    let body_area = Rect {
        x: area.x,
        y: area.y + 1,
        width: area.width,
        height: area.height.saturating_sub(1),
    };

    if app.log.lines.is_empty() {
        let placeholder = Paragraph::new(Line::styled(
            " (no activity)",
            Style::default().fg(palette().subtle),
        ));
        frame.render_widget(placeholder, body_area);
        return;
    }

    // Compute visual rows after wrap to clamp scroll for follow-bottom.
    let total_visual_rows = total_wrapped_rows(&app.log.lines, body_area.width as usize);
    let visible_rows = body_area.height as usize;
    let max_scroll = total_visual_rows.saturating_sub(visible_rows);
    let scroll = if app.log.follow_bottom {
        max_scroll
    } else {
        app.log.scroll.min(max_scroll)
    };

    let log_lines: Vec<Line> = app
        .log
        .lines
        .iter()
        .map(|l| Line::styled(l.as_str(), Style::default().fg(palette().subtle)))
        .collect();

    frame.render_widget(
        Paragraph::new(log_lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll as u16, 0)),
        body_area,
    );
}

fn render_help_overlay(frame: &mut Frame, area: Rect) {
    render_help_overlay_inner(frame, area, false);
}

fn render_help_overlay_pr_detail(frame: &mut Frame, area: Rect) {
    render_help_overlay_inner(frame, area, true);
}

fn render_help_overlay_inner(frame: &mut Frame, area: Rect, pr_detail: bool) {
    let w = HELP_OVERLAY_WIDTH.min(area.width.saturating_sub(4));
    let h = HELP_OVERLAY_HEIGHT.min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let overlay = Rect {
        x,
        y,
        width: w,
        height: h,
    };

    // Clear the overlay area first.
    frame.render_widget(ratatui::widgets::Clear, overlay);

    let lines: Vec<Line> = if pr_detail {
        vec![
            Line::styled(
                " key bindings — PR detail (fullscreen)",
                Style::default().fg(palette().love),
            ),
            Line::styled("─".repeat(w as usize), Style::default().fg(palette().muted)),
            kv_line("  Esc      ", "back to PR list (saves position)"),
            kv_line("  Tab      ", "toggle Files ↔ Diff focus"),
            kv_line("  j / k    ", "Files: prev/next file  ·  Diff: scroll"),
            kv_line("  ] / [    ", "next / prev file (from any focus)"),
            kv_line("  H / L    ", "prev / next hunk (Diff focus only)"),
            kv_line("  r        ", "refresh diff for this PR"),
            kv_line("  o        ", "open PR in browser"),
            kv_line("  1-9      ", "attach to task #N"),
            kv_line("  q        ", "quit orch"),
            Line::styled(
                " Position is saved per PR — re-Enter restores cursor + scroll",
                Style::default().fg(palette().muted),
            ),
        ]
    } else {
        vec![
            Line::styled(" key bindings", Style::default().fg(palette().love)),
            Line::styled("─".repeat(w as usize), Style::default().fg(palette().muted)),
            Line::styled(" Global", Style::default().fg(palette().iris)),
            kv_line("  q        ", "quit"),
            kv_line("  Tab h l  ", "focus list ↔ right"),
            kv_line("  [ ] H L  ", "tasks · detail tabs"),
            kv_line("  1-9      ", "attach to task #N"),
            kv_line("  Esc      ", "right → list; list → quit"),
            kv_line("  C-u/C-d  ", "log scroll  ·  < top  ·  > tail"),
            kv_line("  ? r m a  ", "help · refresh · message · activity"),
            Line::styled(" List", Style::default().fg(palette().iris)),
            kv_line("  j k g G  ", "move · top / bottom"),
            kv_line("  J K      ", "move task down / up in open_order"),
            kv_line("  s p R x  ", "spawn · pause · resume · close"),
            kv_line("  Enter    ", "attach to active pane"),
            Line::styled(" Right zone", Style::default().fg(palette().iris)),
            kv_line("  j k      ", "move cursor in active tab"),
            kv_line("  Enter    ", "open / attach in active tab"),
            Line::styled(
                " Enter on a PR row → fullscreen lazygit-style diff",
                Style::default().fg(palette().muted),
            ),
        ]
    };

    let text_area = Rect {
        x: overlay.x + 1,
        y: overlay.y + 1,
        width: overlay.width.saturating_sub(2),
        height: overlay.height.saturating_sub(2),
    };
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), text_area);

    // Border around overlay.
    use ratatui::widgets::{Block, Borders};
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette().love));
    frame.render_widget(block, overlay);
}

fn kv_line<'a>(key: &'a str, value: &'a str) -> Line<'a> {
    Line::from(vec![
        Span::styled(key, Style::default().fg(palette().muted)),
        Span::styled(value, Style::default().fg(palette().text)),
    ])
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i + 1 >= max {
            out.push('…');
            break;
        }
        out.push(c);
    }
    out
}

fn status_str(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Ready => "ready",
        TaskStatus::Working => "working",
        TaskStatus::Unknown => "unknown",
        TaskStatus::Idle => "idle",
        TaskStatus::Paused => "paused",
        TaskStatus::Attached => "attach",
        TaskStatus::Error => "error",
    }
}

fn status_color(status: TaskStatus) -> Color {
    match status {
        TaskStatus::Ready => palette().pine,
        TaskStatus::Working => palette().foam,
        TaskStatus::Unknown => palette().gold,
        TaskStatus::Paused => palette().iris,
        TaskStatus::Idle | TaskStatus::Attached => palette().muted,
        TaskStatus::Error => palette().love,
    }
}

/// Total visual rows a list of lines occupies after word wrap into a
/// fixed-width workspace. Rough but sufficient for scroll clamping.
fn total_wrapped_rows(lines: &[String], width: usize) -> usize {
    if width == 0 {
        return lines.len();
    }
    lines
        .iter()
        .map(|l| {
            if l.is_empty() {
                1
            } else {
                let n = l.chars().count();
                (n + width - 1) / width
            }
        })
        .sum()
}

// Key handling.
//
// Two-zone focus (List <-> Right). Log is passive and uses global scroll keys.

pub fn handle_key(app: &mut App, key: KeyEvent) {
    if app.show_help {
        // Any key dismisses the help overlay.
        app.show_help = false;
        return;
    }
    if app.message_input.is_some() {
        handle_message_input_key(app, key);
        return;
    }

    // Most keys clear any stale toast on the next keypress. Capture the
    // toast before dispatch — if a handler set a fresh one during the
    // match (e.g. H/L hunk feedback, refresh status), we preserve it.
    let toast_before = app.toast.clone();
    let was_toasted = toast_before.is_some();

    match (key.code, key.modifiers) {
        // Quit semantics: from list, q or Esc quits. From right zone,
        // Esc returns focus to the list (one stable home base).
        (KeyCode::Char('q'), _) => {
            app.should_quit = true;
            return;
        }
        (KeyCode::Esc, _) => {
            // PR drill: pop Detail back to List in one Esc (no stack).
            // Saves position so re-entering restores file_cursor + scroll.
            if app.focus == Pane::Right
                && app.detail_tab == Tab::Prs
                && matches!(app.pr_view, PrView::Detail { .. })
            {
                save_and_exit_pr_detail(app);
                if was_toasted { app.toast = None; }
                return;
            }
            if app.focus == Pane::Right {
                app.focus = Pane::List;
            } else {
                app.should_quit = true;
            }
            if was_toasted { app.toast = None; }
            return;
        }
        (KeyCode::Char('?'), _) => {
            app.show_help = true;
            return;
        }
        // In PR Detail: toggle Files ↔ Diff focus. Otherwise: cycle panes.
        (KeyCode::Tab, _) | (KeyCode::BackTab, _) => {
            if app.detail_tab == Tab::Prs
                && matches!(app.pr_view, PrView::Detail { .. })
            {
                handle_pr_detail_focus_toggle(app);
            } else {
                app.focus = match app.focus {
                    Pane::List => Pane::Right,
                    Pane::Right => Pane::List,
                };
            }
        }
        // 1-9 → attach to task at that 1-based position (matches the
        // `#N` rank shown in render_list).
        (KeyCode::Char(c), _) if ('1'..='9').contains(&c) => {
            let idx = (c as u8 - b'1') as usize;
            if let Some(task) = app.tasks.get(idx) {
                if !task.record.tmux.session_name.is_empty() {
                    attach_session(&task.record.tmux.session_name);
                    app.should_quit = true;
                }
            }
        }
        (KeyCode::Char('m'), _) => {
            app.message_input = Some(String::new());
        }
        (KeyCode::Char('a'), _) => {
            app.show_activity = !app.show_activity;
        }
        // Detail tabs are global view state. Switching them does not
        // pull focus away from the task list.
        (KeyCode::Char('H'), _) if !matches!(app.pr_view, PrView::Detail { .. }) => {
            cycle_detail_tab(app, false);
        }
        (KeyCode::Char('L'), _) if !matches!(app.pr_view, PrView::Detail { .. }) => {
            cycle_detail_tab(app, true);
        }
        // Navigate tasks from any focus — useful for cycling through
        // tasks while staying in a detail tab without
        // needing to Tab/Esc back to the list.
        (KeyCode::Char(']'), _) => {
            // In PR Detail: next file. Otherwise: next task.
            if app.focus == Pane::Right
                && app.detail_tab == Tab::Prs
                && matches!(app.pr_view, PrView::Detail { .. })
            {
                handle_pr_detail_next_file(app);
            } else if app.selected + 1 < app.tasks.len() {
                app.selected += 1;
                app.panes_selected = 0;
                reset_detail_cursor_for_new_task(app);
            }
        }
        (KeyCode::Char('['), _) => {
            if app.focus == Pane::Right
                && app.detail_tab == Tab::Prs
                && matches!(app.pr_view, PrView::Detail { .. })
            {
                handle_pr_detail_prev_file(app);
            } else {
                app.selected = app.selected.saturating_sub(1);
                app.panes_selected = 0;
                reset_detail_cursor_for_new_task(app);
            }
        }
        // Global log controls.
        (KeyCode::PageUp, _) | (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
            app.log.follow_bottom = false;
            app.log.scroll = app.log.scroll.saturating_sub(10);
        }
        (KeyCode::PageDown, _) | (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
            app.log.follow_bottom = false;
            app.log.scroll = app.log.scroll.saturating_add(10);
        }
        (KeyCode::Char('<'), _) => {
            app.log.follow_bottom = false;
            app.log.scroll = 0;
        }
        (KeyCode::Char('>'), _) => {
            app.log.follow_bottom = true;
        }
        _ => match app.focus {
            Pane::List => handle_list_key(app, key),
            Pane::Right => handle_right_key(app, key),
        },
    }

    // Only clear if no handler updated the toast.
    if was_toasted && app.toast == toast_before {
        app.toast = None;
    }
}

fn handle_list_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if app.selected + 1 < app.tasks.len() {
                app.selected += 1;
                app.panes_selected = 0;
                reset_detail_cursor_for_new_task(app);
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.selected = app.selected.saturating_sub(1);
            app.panes_selected = 0;
            reset_detail_cursor_for_new_task(app);
        }
        KeyCode::Char('h') | KeyCode::Left if !matches!(app.pr_view, PrView::Detail { .. }) => {
            app.focus = Pane::List;
        }
        KeyCode::Char('l') | KeyCode::Right if !matches!(app.pr_view, PrView::Detail { .. }) => {
            app.focus = Pane::Right;
        }
        KeyCode::Char('g') => app.selected = 0,
        KeyCode::Char('G') => {
            app.selected = app.tasks.len().saturating_sub(1);
        }
        KeyCode::Enter => {
            // Attach to selected task's tmux session (active pane is
            // whatever tmux had selected last).
            if let Some(task) = app.selected_task() {
                if !task.record.tmux.session_name.is_empty() {
                    attach_session(&task.record.tmux.session_name);
                    app.should_quit = true;
                }
            }
        }
        // Lifecycle ops on the selected task — surface success/failure
        // via toast since the TUI owns the screen.
        KeyCode::Char('s') => lifecycle_op(app, "spawn", lifecycle_spawn),
        KeyCode::Char('R') => lifecycle_op(app, "resume", lifecycle_resume),
        KeyCode::Char('p') => lifecycle_op(app, "pause", lifecycle_pause),
        KeyCode::Char('x') => lifecycle_op(app, "close", lifecycle_close),
        // Reorder the selected task within Registry.open_order.
        // J = move down (swap with next), K = move up (swap with prev).
        // Mirrors vim's "J/K to move line" idiom in many editors.
        KeyCode::Char('J') => reorder_selected(app, 1),
        KeyCode::Char('K') => reorder_selected(app, -1),
        _ => {}
    }
}

fn cycle_detail_tab(app: &mut App, forward: bool) {
    app.detail_tab = if forward {
        app.detail_tab.next()
    } else {
        app.detail_tab.prev()
    };
    if app.detail_tab == Tab::Prs {
        ensure_pr_cursor(app);
    }
}

/// Swap the selected task with its neighbor (delta = +1 down, -1 up)
/// in `Registry.open_order`. Refreshes the task list so the row
/// position updates and the cursor follows the moved task.
fn reorder_selected(app: &mut App, delta: isize) {
    let store = crate::store::Store::default();
    let selected_name = match app.tasks.get(app.selected) {
        Some(t) => t.name.clone(),
        None => return,
    };
    let mut registry = match store.load_registry() {
        Some(r) => r,
        None => {
            app.toast = Some("no registry".into());
            return;
        }
    };
    // Find the selected slug's id by scanning the task records — the
    // open_order indices may differ from the visible task list when
    // closed tasks linger, so go via name → record → id.
    let selected_id = registry
        .open_order
        .iter()
        .copied()
        .find(|id| {
            store
                .load_record(*id)
                .map(|r| r.slug == selected_name)
                .unwrap_or(false)
        });
    let Some(id) = selected_id else {
        app.toast = Some(format!("not in open_order: {selected_name}"));
        return;
    };
    let Some(pos) = registry.open_order.iter().position(|i| *i == id) else {
        return;
    };
    let new_pos = pos as isize + delta;
    if new_pos < 0 || new_pos >= registry.open_order.len() as isize {
        return;
    }
    registry.open_order.swap(pos, new_pos as usize);
    store.save_registry(&registry);
    app.refresh_status();
    // Move the cursor to follow the swapped task so the user sees it
    // moving, not just disappearing into a different row.
    if let Some(new_idx) = app.tasks.iter().position(|t| t.name == selected_name) {
        app.selected = new_idx;
    }
}

fn lifecycle_op(app: &mut App, label: &str, f: fn(&str, &str) -> Result<String, String>) {
    let Some(task) = app.selected_task() else {
        return;
    };
    let name = task.name.clone();
    let session = task.record.tmux.session_name.clone();
    match f(&name, &session) {
        Ok(msg) => app.toast = Some(msg),
        Err(e) => app.toast = Some(format!("{label} failed: {e}")),
    }
    // Re-pull task state so the row badge reflects the change.
    app.refresh_status();
}

/// Spawn a worker for the task, reusing a paused tmux session when present.
fn lifecycle_spawn(name: &str, _session: &str) -> Result<String, String> {
    let store = store::Store::default();
    let record = store
        .load_record_by_slug(name)
        .ok_or_else(|| format!("no task '{name}'"))?;
    let session = if record.tmux.session_name.is_empty() {
        format!("task-{name}")
    } else {
        record.tmux.session_name.clone()
    };
    let task_file = state::tasks_dir().join(format!("{name}.md"));
    if !task_file.exists() {
        return Err(format!("no task file: {}", task_file.display()));
    }
    let allow_existing_dirty = record.desired_state != DesiredState::New;
    let work_dir = state::prepare_task_worktree(
        name,
        &record.worktree.path,
        allow_existing_dirty,
    )?;
    let cmd_str = record.agent.worker_kind.worker_cmd(&task_file);
    let actual_session = find_actual_session(&session);
    let pane_id = if let Some(actual) = &actual_session {
        Some(state::start_worker_in_session(
            actual,
            record.tmux.last_known_pane_id.as_deref(),
            &cmd_str,
        )?)
    } else {
        let new_ok = Command::new("tmux")
            .args(["new-session", "-d", "-s", &session, "-c", &work_dir])
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !new_ok {
            return Err("tmux new-session failed".into());
        }
        let send_ok = Command::new("tmux")
            .args(["send-keys", "-t", &session, &cmd_str, "Enter"])
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !send_ok {
            return Err("tmux send-keys failed".into());
        }
        None
    };
    let now = cache::now_epoch();
    store.update_record_by_slug(name, |r| {
        r.tmux.session_name = session.clone();
        if let Some(pane_id) = &pane_id {
            r.tmux.last_known_pane_id = Some(pane_id.clone());
        }
        r.worktree.path = work_dir.clone();
        r.desired_state = DesiredState::Active;
        r.paused_at = None;
        if r.started_at.is_none() {
            r.started_at = Some(now);
        }
        r.updated_at = now;
    });
    if actual_session.is_some() {
        Ok(format!("resumed {session}"))
    } else {
        Ok(format!("spawned {session}"))
    }
}

fn lifecycle_resume(name: &str, _session: &str) -> Result<String, String> {
    lifecycle_spawn(name, "")
}

fn lifecycle_pause(name: &str, session: &str) -> Result<String, String> {
    let store = store::Store::default();
    let record = store
        .load_record_by_slug(name)
        .ok_or_else(|| format!("no task '{name}'"))?;
    let mut paused_pane = None;
    if !session.is_empty() {
        if let Some(actual) = find_actual_session(session) {
            paused_pane = state::pause_worker_panes(
                &actual,
                &record.worktree.path,
            )?;
        }
    }
    let now = cache::now_epoch();
    store.update_record_by_slug(name, |r| {
        r.desired_state = DesiredState::Paused;
        if let Some(pane_id) = &paused_pane {
            r.tmux.last_known_pane_id = Some(pane_id.clone());
        }
        r.paused_at = Some(now);
        r.updated_at = now;
    });
    Ok(format!("paused {name}; tmux preserved"))
}

fn lifecycle_close(name: &str, session: &str) -> Result<String, String> {
    let store = store::Store::default();
    let Some(record) = store.load_record_by_slug(name) else {
        return Err(format!("no task '{name}'"));
    };
    let id = record.id;
    let worktree_path = record.worktree.path.clone();
    let now = cache::now_epoch();
    let dir = state::tasks_dir();
    let archive_path = dir.join("done").join(format!("{id}-{name}.md"));

    store.update_record_by_slug(name, |r| {
        r.desired_state = DesiredState::Closed;
        r.closed_at = Some(now);
        r.archived_task_file = Some(archive_path.clone());
        r.updated_at = now;
    });
    if let Some(mut registry) = store.load_registry() {
        registry.open_order.retain(|i| *i != id);
        if !registry.closed_order.contains(&id) {
            registry.closed_order.push(id);
        }
        store.save_registry(&registry);
    }

    // 2. Archive .md — abort on failure.
    let md = dir.join(format!("{name}.md"));
    if md.exists() {
        std::fs::create_dir_all(dir.join("done"))
            .map_err(|e| format!("create done/: {e}"))?;
        std::fs::rename(&md, &archive_path)
            .map_err(|e| format!("archive {name}.md: {e}"))?;
    }

    // 3. Kill tmux.
    if !session.is_empty() {
        if let Some(actual) = find_actual_session(session) {
            let _ = Command::new("tmux")
                .args(["kill-session", "-t", &actual])
                .stderr(Stdio::null())
                .status();
        }
    }

    // 4. Remove worktree. Failures persist as drift.cleanup_failed so
    //    the orphan worktree surfaces in `orch status` later — the
    //    transient toast alone is easy to miss.
    let warning = if worktree_path.is_empty() {
        None
    } else {
        let wt = state::expand_home(&worktree_path);
        let path = std::path::Path::new(&wt);
        if path.exists() {
            match state::remove_worktree(path) {
                Ok(()) => None,
                Err(e) => {
                    let err_str = store.mark_worktree_cleanup_failed(name, &e);
                    Some(format!("worktree {wt} not removed: {err_str}"))
                }
            }
        } else {
            None
        }
    };

    Ok(match warning {
        None => format!("closed {name}"),
        Some(w) => format!("closed {name} (WARN: {w})"),
    })
}

/// Right-zone key dispatch. j/k always means "move cursor in active
/// tab"; Enter always means "act on cursored item".
fn handle_right_key(app: &mut App, key: KeyEvent) {
    match (app.detail_tab, key.code) {
        (_, KeyCode::Char('h')) | (_, KeyCode::Left) => {
            app.focus = Pane::List;
        }
        (_, KeyCode::Char('l')) | (_, KeyCode::Right) => {
            app.focus = Pane::Right;
        }
        (Tab::Panes, KeyCode::Char('j')) | (Tab::Panes, KeyCode::Down) => {
            let n = app.selected_task().map(|t| t.panes.len()).unwrap_or(0);
            if app.panes_selected + 1 < n {
                app.panes_selected += 1;
            }
        }
        (Tab::Panes, KeyCode::Char('k')) | (Tab::Panes, KeyCode::Up) => {
            app.panes_selected = app.panes_selected.saturating_sub(1);
        }
        (Tab::Panes, KeyCode::Enter) => {
            if let Some(task) = app.selected_task() {
                if let Some(pane) = task.panes.get(app.panes_selected) {
                    attach_pane(&pane.session, &pane.id);
                    app.should_quit = true;
                }
            }
        }
        // PR tab.
        (Tab::Prs, KeyCode::Char('j')) | (Tab::Prs, KeyCode::Down) => {
            if matches!(app.pr_view, PrView::Detail { .. }) {
                handle_pr_detail_down(app);
            } else {
                handle_pr_down(app);
            }
        }
        (Tab::Prs, KeyCode::Char('k')) | (Tab::Prs, KeyCode::Up) => {
            if matches!(app.pr_view, PrView::Detail { .. }) {
                handle_pr_detail_up(app);
            } else {
                handle_pr_up(app);
            }
        }
        (Tab::Prs, KeyCode::Enter) => {
            handle_pr_enter(app);
        }
        (Tab::Prs, KeyCode::Tab) | (Tab::Prs, KeyCode::BackTab) => {
            // Inside Detail, Tab toggles file/diff focus rather than
            // bouncing back to the task list. Outside Detail, fall
            // through (return without handling so the global Tab fires).
            if matches!(app.pr_view, PrView::Detail { .. }) {
                handle_pr_detail_focus_toggle(app);
            }
        }
        (Tab::Prs, KeyCode::Char('H')) => {
            // Hunk jump: works from any focus; auto-switches to Diff so
            // the scroll change is visible.
            if matches!(app.pr_view, PrView::Detail { .. }) {
                if let PrView::Detail { focus, .. } = &mut app.pr_view {
                    *focus = PrDetailFocus::Diff;
                }
                handle_pr_detail_hunk_jump(app, false);
            }
        }
        (Tab::Prs, KeyCode::Char('L')) => {
            if matches!(app.pr_view, PrView::Detail { .. }) {
                if let PrView::Detail { focus, .. } = &mut app.pr_view {
                    *focus = PrDetailFocus::Diff;
                }
                handle_pr_detail_hunk_jump(app, true);
            }
        }
        (Tab::Prs, KeyCode::Char('r')) => {
            handle_pr_refresh(app);
        }
        (Tab::Prs, KeyCode::Char('o')) => {
            handle_pr_open_browser(app);
        }
        _ => {}
    }
}

fn reset_detail_cursor_for_new_task(app: &mut App) {
    app.pr_view = PrView::default();
}

/// Set the PR cursor to the first linked PR of the selected task. No-op
/// if a cursor is already set or the task has none. Called when entering
/// the PR tab so the preview pane has something to show.
fn ensure_pr_cursor(app: &mut App) {
    let first = app
        .tasks
        .get(app.selected)
        .and_then(|t| t.prs.first().map(|p| p.number));
    if let PrView::List { cursor_number } = &mut app.pr_view {
        if *cursor_number == 0 {
            if let Some(n) = first {
                *cursor_number = n;
            }
        }
    }
}

fn handle_pr_down(app: &mut App) {
    let prs: Vec<u32> = app
        .tasks
        .get(app.selected)
        .map(|t| t.prs.iter().map(|p| p.number).collect())
        .unwrap_or_default();
    if prs.is_empty() {
        return;
    }
    let cur = match &app.pr_view {
        PrView::List { cursor_number } => *cursor_number,
        _ => return,
    };
    let pos = prs.iter().position(|n| *n == cur).unwrap_or(0);
    let next = if cur == 0 {
        prs[0]
    } else if pos + 1 < prs.len() {
        prs[pos + 1]
    } else {
        cur
    };
    app.pr_view = PrView::List { cursor_number: next };
}

fn handle_pr_up(app: &mut App) {
    let prs: Vec<u32> = app
        .tasks
        .get(app.selected)
        .map(|t| t.prs.iter().map(|p| p.number).collect())
        .unwrap_or_default();
    if prs.is_empty() {
        return;
    }
    let cur = match &app.pr_view {
        PrView::List { cursor_number } => *cursor_number,
        _ => return,
    };
    let pos = prs.iter().position(|n| *n == cur).unwrap_or(0);
    let prev = if cur == 0 {
        prs[0]
    } else if pos > 0 {
        prs[pos - 1]
    } else {
        cur
    };
    app.pr_view = PrView::List { cursor_number: prev };
}

/// Enter on a PR row → drill into Detail. Lazy-fetches the diff when the
/// cache entry is missing or the head_sha doesn't match the PR metadata
/// (force-push detection). Restores `(file_cursor, scroll)` from a prior
/// drill into the same PR so re-entering doesn't lose your position.
fn handle_pr_enter(app: &mut App) {
    let cur = match &app.pr_view {
        PrView::List { cursor_number } if *cursor_number > 0 => *cursor_number,
        _ => return,
    };
    fetch_diff_if_needed(cur);
    let (file_cursor, scroll) =
        app.pr_detail_state.get(&cur).copied().unwrap_or((0, 0));
    app.pr_view = PrView::Detail {
        number: cur,
        focus: PrDetailFocus::Files,
        file_cursor,
        scroll,
    };
}

/// Persist the drilled PR's position into `app.pr_detail_state` and
/// reset `pr_view` to `List`. No-op when not drilled.
fn save_and_exit_pr_detail(app: &mut App) {
    if let PrView::Detail { number, file_cursor, scroll, .. } = &app.pr_view {
        app.pr_detail_state.insert(*number, (*file_cursor, *scroll));
        let cursor = *number;
        app.pr_view = PrView::List { cursor_number: cursor };
    }
}

/// Force-refresh the diff cache for the currently-drilled PR. Spawns
/// the fetch off the UI thread; the cache file is the contract — the
/// next render reads it and reflects the new state.
fn handle_pr_refresh(app: &mut App) {
    let Some(n) = (match &app.pr_view {
        PrView::Detail { number, .. } => Some(*number),
        _ => None,
    }) else {
        return;
    };
    app.toast = Some(format!("refreshing #{n}…"));
    spawn_pr_diff_fetch(n);
}

/// Lazy diff fetch: only fetches when the cache is missing or the PR's
/// `head_sha` has moved past what was cached. Spawns when work is
/// needed; safe to call on every Enter.
fn fetch_diff_if_needed(number: u32) {
    let pr_cache = crate::cache::read_prs();
    let live_sha = pr_cache
        .prs
        .get(&number)
        .map(|p| p.head_sha.clone())
        .unwrap_or_default();
    let diff_cache = crate::cache::read_pr_diffs();
    let needs_fetch = match diff_cache.diffs.get(&number) {
        Some(d) => !live_sha.is_empty() && d.head_sha != live_sha,
        None => true,
    };
    if !needs_fetch {
        return;
    }
    spawn_pr_diff_fetch(number);
}

fn spawn_pr_diff_fetch(number: u32) {
    std::thread::spawn(move || {
        let pr_cache = crate::cache::read_prs();
        let head_sha = pr_cache
            .prs
            .get(&number)
            .map(|p| p.head_sha.clone())
            .unwrap_or_default();
        let diff = crate::gh::fetch_pr_diff(number, &head_sha);
        let mut cache = crate::cache::read_pr_diffs();
        cache.diffs.insert(number, diff);
        cache.generated_at = crate::cache::now_epoch();
        crate::cache::write_pr_diffs(&cache);
    });
}

/// Clamp `file_cursor` against the current diff's file count. Called
/// after fetch returns and before each render to handle the case where
/// a refresh produced fewer files than before.
fn clamp_pr_file_cursor(app: &mut App) {
    if let PrView::Detail { number, file_cursor, scroll, .. } = &mut app.pr_view {
        let n = file_count(*number);
        if n == 0 {
            *file_cursor = 0;
            *scroll = 0;
            return;
        }
        if *file_cursor >= n {
            *file_cursor = n - 1;
            *scroll = 0;
        }
    }
}

fn handle_pr_detail_down(app: &mut App) {
    if let PrView::Detail { number, focus, file_cursor, scroll } = &mut app.pr_view {
        match focus {
            PrDetailFocus::Files => {
                let n_files = file_count(*number);
                if *file_cursor + 1 < n_files {
                    *file_cursor += 1;
                    *scroll = 0;
                }
            }
            PrDetailFocus::Diff => {
                *scroll = scroll.saturating_add(1);
            }
        }
    }
}

fn handle_pr_detail_up(app: &mut App) {
    if let PrView::Detail { focus, file_cursor, scroll, .. } = &mut app.pr_view {
        match focus {
            PrDetailFocus::Files => {
                if *file_cursor > 0 {
                    *file_cursor -= 1;
                    *scroll = 0;
                }
            }
            PrDetailFocus::Diff => {
                *scroll = scroll.saturating_sub(1);
            }
        }
    }
}

fn handle_pr_detail_next_file(app: &mut App) {
    if let PrView::Detail { number, file_cursor, scroll, .. } = &mut app.pr_view {
        let n_files = file_count(*number);
        if *file_cursor + 1 < n_files {
            *file_cursor += 1;
            *scroll = 0;
        }
    }
}

fn handle_pr_detail_prev_file(app: &mut App) {
    if let PrView::Detail { file_cursor, scroll, .. } = &mut app.pr_view {
        if *file_cursor > 0 {
            *file_cursor -= 1;
            *scroll = 0;
        }
    }
}

fn handle_pr_detail_focus_toggle(app: &mut App) {
    if let PrView::Detail { focus, .. } = &mut app.pr_view {
        *focus = match *focus {
            PrDetailFocus::Files => PrDetailFocus::Diff,
            PrDetailFocus::Diff => PrDetailFocus::Files,
        };
    }
}

fn handle_pr_detail_hunk_jump(app: &mut App, forward: bool) {
    let (number, file_cursor_val, cur_scroll) = match &app.pr_view {
        PrView::Detail { number, file_cursor, scroll, .. } => (*number, *file_cursor, *scroll),
        _ => return,
    };
    let cache = crate::cache::read_pr_diffs();
    let Some(diff) = cache.diffs.get(&number) else {
        app.toast = Some("no diff cached — press r to fetch".into());
        return;
    };
    let Some(file) = diff.files.get(file_cursor_val) else {
        return;
    };
    let (_, hunk_anchors) = build_pr_diff_lines(file, 80);
    let n = hunk_anchors.len();
    if n == 0 {
        app.toast = Some("no hunks (empty or binary)".into());
        return;
    }

    // Current hunk index — last anchor at or before cur_scroll.
    let cur_idx = hunk_anchors
        .iter()
        .rposition(|r| *r <= cur_scroll)
        .unwrap_or(0);

    let target_idx = if forward {
        if cur_idx + 1 < n { Some(cur_idx + 1) } else { None }
    } else if cur_idx > 0 {
        Some(cur_idx - 1)
    } else {
        None
    };

    let Some(idx) = target_idx else {
        app.toast = Some(if forward {
            format!("last hunk ({}/{n})", cur_idx + 1)
        } else {
            format!("first hunk ({}/{n})", cur_idx + 1)
        });
        return;
    };

    let new_scroll = hunk_anchors[idx];
    if let PrView::Detail { scroll, .. } = &mut app.pr_view {
        *scroll = new_scroll;
    }
    app.toast = Some(format!("hunk {}/{n}", idx + 1));
}

fn file_count(number: u32) -> usize {
    let cache = crate::cache::read_pr_diffs();
    cache.diffs.get(&number).map(|d| d.files.len()).unwrap_or(0)
}

fn handle_pr_open_browser(app: &App) {
    let n = match &app.pr_view {
        PrView::List { cursor_number } if *cursor_number > 0 => *cursor_number,
        PrView::Detail { number, .. } => *number,
        _ => return,
    };
    let _ = std::process::Command::new("gh")
        .args(["pr", "view", &n.to_string(), "--web"])
        .stderr(Stdio::null())
        .stdout(Stdio::null())
        .status();
}

fn handle_message_input_key(app: &mut App, key: KeyEvent) {
    let Some(buf) = app.message_input.as_mut() else {
        return;
    };
    match key.code {
        KeyCode::Esc => {
            app.message_input = None;
        }
        KeyCode::Enter => {
            let msg = std::mem::take(buf);
            app.message_input = None;
            if !msg.trim().is_empty() && !app.readonly {
                send_message(&msg);
            }
        }
        KeyCode::Backspace => {
            buf.pop();
        }
        KeyCode::Char(c) => {
            buf.push(c);
        }
        _ => {}
    }
}

fn attach_session(session: &str) {
    attach(session, None);
}

fn attach_pane(session: &str, pane_id: &str) {
    attach(session, Some(pane_id));
}

fn attach(session: &str, pane_id: Option<&str>) {
    let Some(actual) = find_actual_session(session) else {
        return;
    };
    let in_tmux = std::env::var("TMUX").is_ok();
    let action = if in_tmux { "switch-client" } else { "attach-session" };
    let _ = Command::new("tmux").args([action, "-t", &actual]).status();
    if let Some(pane) = pane_id {
        let _ = Command::new("tmux")
            .args(["select-pane", "-t", pane])
            .stderr(Stdio::null())
            .status();
    }
    kill_source_pane();
}

/// Tmux pane id orch is running in. `$TMUX_PANE` first, then a tty/pane-list
/// match (some launchers filter env vars). Eagerly warmed in `run()` before
/// raw mode since the `tty` query needs a normal stdin.
static SOURCE_PANE: OnceLock<Option<String>> = OnceLock::new();

fn resolve_source_pane() -> Option<String> {
    if let Ok(p) = std::env::var("TMUX_PANE") {
        if !p.is_empty() {
            return Some(p);
        }
    }
    // `tty` needs to inherit our stdin (the pty) — Rust's default for
    // Command::output() is a piped stdin, which would make tty report
    // "not a tty" and exit 1.
    let tty_out = Command::new("tty")
        .stdin(Stdio::inherit())
        .output()
        .ok()?;
    if !tty_out.status.success() {
        return None;
    }
    let our_tty = String::from_utf8_lossy(&tty_out.stdout).trim().to_string();
    if our_tty.is_empty() {
        return None;
    }
    let panes = Command::new("tmux")
        .args(["list-panes", "-a", "-F", "#{pane_id} #{pane_tty}"])
        .output()
        .ok()?;
    if !panes.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&panes.stdout);
    for line in s.lines() {
        let mut parts = line.splitn(2, ' ');
        let id = parts.next()?;
        let pane_tty = parts.next()?.trim();
        if pane_tty == our_tty {
            return Some(id.to_string());
        }
    }
    None
}

fn source_pane() -> Option<&'static str> {
    SOURCE_PANE
        .get_or_init(resolve_source_pane)
        .as_deref()
}

// Outside tmux, `attach-session` runs tmux itself as the foreground process.
// No separate source pane to clean up, so skip.
fn kill_source_pane() {
    if std::env::var("TMUX").is_err() {
        return;
    }
    let Some(pane) = source_pane() else { return };
    let _ = Command::new("tmux")
        .args(["kill-pane", "-t", pane])
        .stderr(Stdio::null())
        .status();
}

fn send_message(msg: &str) {
    let dir = state::tasks_dir().join(".inbox");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = dir.join(format!("{nanos}-{}.msg", std::process::id()));
    let _ = std::fs::write(path, msg);
}

// Debug rendering — dumps the current TUI to stdout at a fixed size.
// Useful for diagnosing layout without an interactive terminal.

pub fn render_debug(width: u16, height: u16, tab: &str, focus: &str, select: usize) {
    use ratatui::backend::TestBackend;
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("debug backend");
    let mut app = App::new();
    app.detail_tab = match tab.to_lowercase().as_str() {
        "prs" => Tab::Prs,
        "panes" => Tab::Panes,
        _ => Tab::Overview,
    };
    app.focus = match focus.to_lowercase().as_str() {
        "details" | "right" => Pane::Right,
        "log" => Pane::Right,
        _ => Pane::List,
    };
    if select < app.tasks.len() {
        app.selected = select;
    }
    terminal.draw(|f| render(f, &mut app)).expect("debug draw");
    let buffer = terminal.backend().buffer().clone();
    for y in 0..height {
        for x in 0..width {
            print!("{}", buffer[(x, y)].symbol());
        }
        println!();
    }
}

// Run loop.

pub fn run() -> io::Result<()> {
    // Resolve our tmux pane id before raw mode kicks in (tty query needs
    // a normal stdin handle).
    let _ = source_pane();
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    let mut app = App::new();

    let res: io::Result<()> = (|| {
        while !app.should_quit {
            terminal.draw(|f| render(f, &mut app))?;

            if event::poll(Duration::from_millis(200))? {
                if let Event::Key(key) = event::read()? {
                    handle_key(&mut app, key);
                }
            }

            if app.last_fast.elapsed() >= FAST_TICK {
                app.last_fast = Instant::now();
                app.refresh_status();
                app.refresh_log();
            }
        }
        Ok(())
    })();

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    res
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    fn test_tasks() -> Vec<TaskView> {
        vec![
            TaskView {
                name: "infra-triage".into(),
                record: TaskRecord {
                    id: 2,
                    slug: "infra-triage".into(),
                    tmux: store::TmuxInfo {
                        session_name: "task-infra-triage".into(),
                        ..Default::default()
                    },
                    worktree: store::WorktreeInfo {
                        path: "/Users/a/work/repo/task-infra-triage".into(),
                        ..Default::default()
                    },
                    desired_state: DesiredState::Active,
                    links: store::Links {
                        prs: vec![store::PrLink {
                            number: 25163,
                            ..Default::default()
                        }],
                        ..Default::default()
                    },
                    ..Default::default()
                },
                status: TaskStatus::Working,
                prs: vec![state::PrData {
                    number: 25163,
                    title: "Fix bene-matching boundary".into(),
                    ci_pass: Some(true),
                    approved: true,
                    ..Default::default()
                }],
                panes: vec![
                    TmuxPaneInfo {
                        id: "%1".into(),
                        session: "task-infra-triage".into(),
                        command: "claude".into(),
                        active: true,
                    },
                    TmuxPaneInfo {
                        id: "%2".into(),
                        session: "task-infra-triage".into(),
                        command: "jj".into(),
                        active: false,
                    },
                ],
            },
            TaskView {
                name: "ach-sanitize".into(),
                record: TaskRecord {
                    id: 3,
                    slug: "ach-sanitize".into(),
                    tmux: store::TmuxInfo {
                        session_name: "task-ach-sanitize".into(),
                        ..Default::default()
                    },
                    desired_state: DesiredState::Paused,
                    ..Default::default()
                },
                status: TaskStatus::Paused,
                prs: vec![],
                panes: vec![],
            },
            TaskView {
                name: "fresh-task".into(),
                record: TaskRecord {
                    id: 4,
                    slug: "fresh-task".into(),
                    ..Default::default()
                },
                status: TaskStatus::Idle,
                prs: vec![],
                panes: vec![],
            },
        ]
    }

    fn test_app() -> App {
        App {
            tasks: test_tasks(),
            selected: 0,
            focus: Pane::List,
            detail_tab: Tab::Overview,
            panes_selected: 0,
            pr_view: PrView::default(),
            pr_detail_state: HashMap::new(),
            log: LogPane {
                run_id: Some("1234-test".into()),
                lines: vec![
                    "[scan] starting".into(),
                    "checking 3 tasks".into(),
                    "".into(),
                    "infra-triage: working".into(),
                ],
                scroll: 0,
                follow_bottom: true,
                last_len: 100,
                finished: false,
            },
            show_activity: false,
            show_help: false,
            daemon_alive: true,
            last_fast: Instant::now(),
            should_quit: false,
            message_input: None,
            read_runs: HashSet::new(),
            last_run_count: 0,
            toast: None,
            readonly: true,
        }
    }

    #[test]
    fn shortcut_hint_tracks_selected_task_lifecycle() {
        let mut app = test_app();

        assert_eq!(
            shortcut_hint(&app),
            " j/k move · Enter attach · p pause · ? keys"
        );
        app.selected = 1;
        assert_eq!(
            shortcut_hint(&app),
            " j/k move · Enter attach · R resume · ? keys"
        );
        app.selected = 2;
        assert_eq!(shortcut_hint(&app), " j/k move · s start · ? keys");

        app.selected = 0;
        app.tasks[0].status = TaskStatus::Paused;
        assert_eq!(
            shortcut_hint(&app),
            " j/k move · Enter attach · R resume · ? keys"
        );
    }

    #[test]
    fn shortcut_hint_tracks_detail_tab_actions() {
        let mut app = test_app();
        app.focus = Pane::Right;

        assert_eq!(shortcut_hint(&app), " H/L tabs · Esc tasks · ? keys");
        app.detail_tab = Tab::Prs;
        assert_eq!(
            shortcut_hint(&app),
            " j/k move · Enter diff · o open · H/L tabs · Esc tasks · ? keys"
        );
        app.detail_tab = Tab::Panes;
        assert_eq!(
            shortcut_hint(&app),
            " j/k move · Enter attach · H/L tabs · Esc tasks · ? keys"
        );
    }

    #[test]
    fn next_selection_follows_name_when_present() {
        let tasks = test_tasks();
        // Cursor was on tasks[1], now the same task is at index 2 — follow it.
        let mut reordered = tasks.clone();
        reordered.swap(0, 1);
        let name = tasks[1].name.clone();
        assert_eq!(next_selection(1, Some(&name), &reordered), 0);
    }

    #[test]
    fn next_selection_clamps_to_position_when_name_gone() {
        let mut shrunk = test_tasks();
        let closed_name = shrunk.remove(0).name;
        assert_eq!(next_selection(0, Some(&closed_name), &shrunk), 0);
        // Cursor on the last task — clamp to new len-1.
        let mut shrunk = test_tasks();
        let last = shrunk.len() - 1;
        let closed_name = shrunk.remove(last).name;
        assert_eq!(next_selection(last, Some(&closed_name), &shrunk), shrunk.len() - 1);
    }

    #[test]
    fn next_selection_empty_list() {
        assert_eq!(next_selection(0, Some("foo"), &[]), 0);
        assert_eq!(next_selection(5, None, &[]), 0);
    }

    fn render_to_string(app: &App, w: u16, h: u16) -> String {
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut owned = app.clone();
        terminal.draw(|f| render(f, &mut owned)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..h {
            for x in 0..w {
                out.push_str(buffer[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn snapshot_three_pane_base() {
        let app = test_app();
        let s = render_to_string(&app, 100, 25);
        // Left pane: tasks list.
        assert!(s.contains("tasks"));
        assert!(s.contains("infra-triage"));
        assert!(s.contains("ach-sanitize"));
        // Right top: tab bar with Overview selected.
        assert!(s.contains("Overview"));
        assert!(s.contains("PRs 1"));
        assert!(s.contains("Panes 2"));
        assert!(!s.contains("Linear"));
        assert!(s.contains("j/k move · Enter attach"));
        // Activity is hidden by default.
        assert!(!s.contains("activity"));
        assert!(!s.contains("infra-triage: working"));
    }

    #[test]
    fn snapshot_three_pane_list_focus() {
        let mut app = test_app();
        app.focus = Pane::List;
        app.selected = 0;
        let s = render_to_string(&app, 100, 25);
        // Selected row has the focus marker; positional rank (#1 because
        // infra-triage is first in the test fixture's open_order) is
        // rendered before the name.
        assert!(s.contains("▸ #1 infra-triage"));
    }

    #[test]
    fn snapshot_three_pane_details_focus() {
        let mut app = test_app();
        app.focus = Pane::Right;
        let s = render_to_string(&app, 100, 25);
        // Tab "Overview" should still be the active tab.
        assert!(s.contains("Overview"));
    }

    #[test]
    fn snapshot_activity_visible() {
        let mut app = test_app();
        app.focus = Pane::Right;
        app.show_activity = true;
        let s = render_to_string(&app, 100, 25);
        assert!(s.contains("activity"));
        assert!(s.contains("infra-triage: working"));
    }

    #[test]
    fn snapshot_detail_tab_overview() {
        let mut app = test_app();
        app.focus = Pane::Right;
        app.detail_tab = Tab::Overview;
        let s = render_to_string(&app, 100, 25);
        assert!(s.contains("infra-triage"));
        assert!(s.contains("session"));
        assert!(s.contains("worktree"));
        assert!(s.contains("task-infra-triage"));
    }

    #[test]
    fn display_worktree_path_strips_repo_prefix() {
        assert_eq!(
            display_worktree_path_with_repo(
                "/Users/a/work/repo/task-create-cancel-sar-filing-endpoints",
                Some("/Users/a/work/repo"),
            ),
            "task-create-cancel-sar-filing-endpoints",
        );
    }

    #[test]
    fn display_worktree_path_falls_back_to_name() {
        assert_eq!(
            display_worktree_path_with_repo(
                "/Users/a/other/task-create-cancel-sar-filing-endpoints",
                Some("/Users/a/work/repo"),
            ),
            "task-create-cancel-sar-filing-endpoints",
        );
    }

    #[test]
    fn snapshot_detail_tab_prs() {
        let mut app = test_app();
        app.focus = Pane::Right;
        app.detail_tab = Tab::Prs;
        let s = render_to_string(&app, 100, 25);
        assert!(s.contains("#25163"));
        assert!(s.contains("Fix bene-matching boundary"));
        assert!(s.contains("ci"));
    }

    #[test]
    fn snapshot_detail_tab_panes() {
        let mut app = test_app();
        app.focus = Pane::Right;
        app.detail_tab = Tab::Panes;
        let s = render_to_string(&app, 100, 25);
        assert!(s.contains("%1"));
        assert!(s.contains("%2"));
        assert!(s.contains("claude"));
        assert!(s.contains("j/k navigate"));
    }

    #[test]
    fn snapshot_detail_tab_panes_selection() {
        let mut app = test_app();
        app.focus = Pane::Right;
        app.detail_tab = Tab::Panes;
        app.panes_selected = 1;
        let s = render_to_string(&app, 100, 25);
        // Second pane (jj) is now selected
        assert!(s.contains("jj"));
    }

    #[test]
    fn snapshot_log_wrapped_lines() {
        let mut app = test_app();
        app.focus = Pane::Right;
        app.show_activity = true;
        app.log.lines = vec![
            "this is a really long log line that absolutely will not fit in 60 workspaces of width and must wrap onto multiple visual rows when rendered".into(),
            "".into(),
            "[scan] short line".into(),
        ];
        // Use 60 cols of total width — log pane gets ~60 of right workspace.
        let s = render_to_string(&app, 100, 25);
        // Should contain the start of the long line and some tail content
        // (if it didn't wrap, the tail would be off-screen).
        assert!(s.contains("this is a really long"));
        assert!(s.contains("[scan] short line"));
    }

    #[test]
    fn snapshot_log_scroll_preserved() {
        let mut app = test_app();
        app.focus = Pane::Right;
        app.show_activity = true;
        app.log.lines = (0..30).map(|i| format!("line {i}")).collect();
        app.log.scroll = 5;
        app.log.follow_bottom = false;

        let s = render_to_string(&app, 100, 25);
        // line 5 should be near the top of the log pane
        assert!(s.contains("line 5"));
        // line 0 should be scrolled off
        assert!(!s.lines().any(|l| l.contains(" line 0 ")));
    }

    #[test]
    fn snapshot_empty_state_no_tasks() {
        let mut app = test_app();
        app.tasks.clear();
        let s = render_to_string(&app, 100, 25);
        assert!(s.contains("no tasks"));
        assert!(s.contains("select a task"));
    }

    #[test]
    fn snapshot_key_help_overlay() {
        let mut app = test_app();
        app.show_help = true;
        let s = render_to_string(&app, 100, 25);
        assert!(s.contains("key bindings"));
        assert!(s.contains("focus list ↔ right"));
        assert!(s.contains("tasks · detail tabs"));
        assert!(s.contains("attach to task #N"));
        assert!(s.contains("activity"));
        assert!(s.contains("Enter on a PR row"));
    }

    #[test]
    fn pr_detail_hunk_jump_uses_real_anchors() {
        // Build a synthetic diff cache with two hunks so H/L can move
        // scroll between known anchor rows.
        use crate::cache::{CachedPrDiff, CachedPrDiffFile, CachedPrDiffHunk, PrDiffCache};
        let dir = std::env::temp_dir().join("orch-test-hunk-jump");
        // Best-effort isolation — the live cache is at ~/tasks/.orch/cache.
        // We only verify the build_pr_diff_lines anchor math, not the
        // actual TUI key dispatch read of the cache.
        let _ = dir;

        let file = CachedPrDiffFile {
            path: "x.go".into(),
            old_path: None,
            additions: 2,
            deletions: 1,
            status: "modified".into(),
            hunks: vec![
                CachedPrDiffHunk {
                    header: "@@ -1,3 +1,4 @@ first".into(),
                    lines: vec![" a".into(), "-b".into(), "+c".into()],
                },
                CachedPrDiffHunk {
                    header: "@@ -10,2 +10,3 @@ second".into(),
                    lines: vec![" d".into(), "+e".into()],
                },
            ],
        };
        let (lines, anchors) = build_pr_diff_lines(&file, 80);
        // Anchors point at the hunk-header line indices.
        assert_eq!(anchors.len(), 2);
        // Both anchors must land on rows that are actually hunk headers.
        let first_header = lines.get(anchors[0] as usize).unwrap();
        let first_text: String = first_header.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(first_text.contains("@@ -1,3"));
        let second_header = lines.get(anchors[1] as usize).unwrap();
        let second_text: String = second_header.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(second_text.contains("@@ -10,2"));
        // Anchor 1 is strictly after anchor 0.
        assert!(anchors[1] > anchors[0]);

        let _ = PrDiffCache::default();
        let _ = CachedPrDiff::default();
    }

    #[test]
    fn pr_detail_tab_toggles_focus_both_ways() {
        let mut app = test_app();
        app.focus = Pane::Right;
        app.detail_tab = Tab::Prs;
        app.pr_view = PrView::Detail {
            number: 4821,
            focus: PrDetailFocus::Files,
            file_cursor: 0,
            scroll: 0,
        };

        handle_key(&mut app, KeyEvent::from(KeyCode::Tab));
        match &app.pr_view {
            PrView::Detail { focus: PrDetailFocus::Diff, .. } => {}
            other => panic!("expected Diff focus, got {other:?}"),
        }

        handle_key(&mut app, KeyEvent::from(KeyCode::Tab));
        match &app.pr_view {
            PrView::Detail { focus: PrDetailFocus::Files, .. } => {}
            other => panic!("expected Files focus, got {other:?}"),
        }
    }

    #[test]
    fn snapshot_help_overlay_pr_detail_variant() {
        let mut app = test_app();
        app.show_help = true;
        app.focus = Pane::Right;
        app.detail_tab = Tab::Prs;
        app.pr_view = PrView::Detail {
            number: 4821,
            focus: PrDetailFocus::Files,
            file_cursor: 0,
            scroll: 0,
        };
        let s = render_to_string(&app, 100, 25);
        assert!(s.contains("PR detail"));
        assert!(s.contains("Tab"));
        assert!(s.contains("hunk"));
        assert!(!s.contains("Phase 1F+"));
    }

    #[test]
    fn wrap_text_preserves_blank_lines() {
        let s = "line one is here\n\nline two is also here";
        let out = wrap_text(s, 10);
        // "line one" wraps; blank line preserved; "line two" wraps
        assert!(out.contains(&String::new()));
        assert!(out.iter().any(|l| l.starts_with("line")));
    }

    #[test]
    fn relative_age_formats() {
        // We can't pin an exact value (depends on now), but sanity-check
        // empty input → empty string.
        assert_eq!(relative_age(""), "");
        // Naive too-short input returns empty
        assert_eq!(relative_age("invalid"), "");
    }

    #[test]
    fn tab_cycling_next_prev() {
        let mut app = test_app();
        app.focus = Pane::List;
        assert_eq!(app.detail_tab, Tab::Overview);
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('L')));
        assert_eq!(app.detail_tab, Tab::Prs);
        assert_eq!(app.focus, Pane::List);
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('L')));
        assert_eq!(app.detail_tab, Tab::Panes);
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('L')));
        assert_eq!(app.detail_tab, Tab::Overview); // wrap
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('H')));
        assert_eq!(app.detail_tab, Tab::Panes); // wrap back
    }

    #[test]
    fn horizontal_navigation_changes_focus_without_changing_tab() {
        let mut app = test_app();
        app.detail_tab = Tab::Prs;

        handle_key(&mut app, KeyEvent::from(KeyCode::Char('l')));
        assert_eq!(app.focus, Pane::Right);
        assert_eq!(app.detail_tab, Tab::Prs);

        handle_key(&mut app, KeyEvent::from(KeyCode::Char('h')));
        assert_eq!(app.focus, Pane::List);
        assert_eq!(app.detail_tab, Tab::Prs);
    }

    #[test]
    fn pane_focus_two_state_toggle() {
        let mut app = test_app();
        assert_eq!(app.focus, Pane::List);
        handle_key(&mut app, KeyEvent::from(KeyCode::Tab));
        assert_eq!(app.focus, Pane::Right);
        handle_key(&mut app, KeyEvent::from(KeyCode::Tab));
        assert_eq!(app.focus, Pane::List);
    }

    #[test]
    fn esc_from_right_returns_to_list() {
        let mut app = test_app();
        app.focus = Pane::Right;
        handle_key(&mut app, KeyEvent::from(KeyCode::Esc));
        assert_eq!(app.focus, Pane::List);
        assert!(!app.should_quit);

        // From list, Esc quits.
        handle_key(&mut app, KeyEvent::from(KeyCode::Esc));
        assert!(app.should_quit);
    }

    #[test]
    #[allow(non_snake_case)]
    fn list_navigation_j_k_g_G() {
        let mut app = test_app();
        assert_eq!(app.selected, 0);
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('j')));
        assert_eq!(app.selected, 1);
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('G')));
        assert_eq!(app.selected, 2);
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('g')));
        assert_eq!(app.selected, 0);
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('k')));
        // Already at top; saturating_sub keeps it at 0
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn panes_tab_jk_navigation() {
        let mut app = test_app();
        app.focus = Pane::Right;
        app.detail_tab = Tab::Panes;
        app.panes_selected = 0;
        // j moves down within panes
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('j')));
        assert_eq!(app.panes_selected, 1);
        // Clamps at the end (only 2 panes in fixture)
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('j')));
        assert_eq!(app.panes_selected, 1);
        // k moves up
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('k')));
        assert_eq!(app.panes_selected, 0);
    }

    #[test]
    fn log_scroll_via_global_keys() {
        // Log is no longer a focus zone. Ctrl-U/Ctrl-D/`<`/`>` work
        // from any focus; PgUp/PgDn remain aliases.
        let mut app = test_app();
        app.focus = Pane::List;
        app.log.follow_bottom = true;
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
        );
        assert!(!app.log.follow_bottom);
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
        );
        handle_key(&mut app, KeyEvent::from(KeyCode::PageUp));
        handle_key(&mut app, KeyEvent::from(KeyCode::PageDown));
        // > re-enables follow_bottom (tail-follow)
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('>')));
        assert!(app.log.follow_bottom);
        // < scrolls to top
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('<')));
        assert!(!app.log.follow_bottom);
        assert_eq!(app.log.scroll, 0);
    }

    #[test]
    fn activity_toggles_from_any_focus() {
        let mut app = test_app();
        assert!(!app.show_activity);

        handle_key(&mut app, KeyEvent::from(KeyCode::Char('a')));
        assert!(app.show_activity);

        app.focus = Pane::Right;
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('a')));
        assert!(!app.show_activity);
    }

    #[test]
    fn transient_feedback_renders_when_activity_hidden() {
        let mut app = test_app();
        app.toast = Some("task paused".into());

        assert!(render_to_string(&app, 100, 25).contains("task paused"));
    }

    #[test]
    fn help_overlay_dismisses_on_any_key() {
        let mut app = test_app();
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('?')));
        assert!(app.show_help);
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('j')));
        assert!(!app.show_help);
    }

    #[test]
    fn message_input_captures_text() {
        let mut app = test_app();
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('m')));
        assert!(app.message_input.is_some());
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('h')));
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('i')));
        assert_eq!(app.message_input.as_deref(), Some("hi"));
        handle_key(&mut app, KeyEvent::from(KeyCode::Esc));
        assert!(app.message_input.is_none());
    }

    #[test]
    fn total_wrapped_rows_handles_long_and_empty() {
        let lines = vec!["short".to_string(), "".to_string(), "x".repeat(100)];
        // Width 50: "short"=1, ""=1, 100/50=2  -> total 4
        assert_eq!(total_wrapped_rows(&lines, 50), 4);
        // Width 0 falls back to line count
        assert_eq!(total_wrapped_rows(&lines, 0), 3);
    }

    #[test]
    fn truncate_handles_short_and_long() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("a longer string", 8), "a longe…");
    }
}
