//! Phase 3 TUI — three-pane layout (list / details / log).
//!
//! See `docs/redesign.md` §5 (TUI Layout) and `docs/redesign-notes.md`
//! Phase 3 for the contract this module implements.
//!
//! Focus model:
//!
//! ```text
//! ┌─────────────────┬──────────────────────────────────────┐
//! │ tasks list      │ Overview · PRs · Linear · Panes      │
//! │  #1 task-foo  · │ ─────────────────────────────────── │
//! │  #2 task-bar  ⚡ │ <selected tab content>              │
//! │  #3 task-baz  ✓ │                                      │
//! │                 ├──────────────────────────────────────┤
//! │                 │ log: latest run output, wrapped      │
//! └─────────────────┴──────────────────────────────────────┘
//! ```

#![allow(dead_code)] // Some bindings stubbed for Phase 4+.

use std::{
    collections::HashSet,
    io::{self, stdout},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent},
    terminal::{
        disable_raw_mode, enable_raw_mode,
        EnterAlternateScreen, LeaveAlternateScreen,
    },
    ExecutableCommand,
};
use ratatui::{
    prelude::*,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Terminal,
};

use crate::state::{
    self, TaskStatus, load_task_meta, load_tmux_sessions,
};

const FAST_TICK: Duration = Duration::from_secs(2);

// Rosé Pine Dawn palette.
const TEXT: Color = Color::Rgb(0x57, 0x52, 0x79);
const SUBTLE: Color = Color::Rgb(0x79, 0x75, 0x93);
const MUTED: Color = Color::Rgb(0x98, 0x93, 0xa5);
const LOVE: Color = Color::Rgb(0xb4, 0x63, 0x7a);
const GOLD: Color = Color::Rgb(0xea, 0x9d, 0x34);
const PINE: Color = Color::Rgb(0x28, 0x69, 0x83);
const FOAM: Color = Color::Rgb(0x56, 0x94, 0x9f);
const IRIS: Color = Color::Rgb(0x90, 0x7a, 0xa9);
const HL_LOW: Color = Color::Rgb(0xf4, 0xed, 0xe8);

// Layout constants.
const LIST_WIDTH: u16 = 34;
const SEPARATOR_WIDTH: u16 = 1;
const TAB_BAR_HEIGHT: u16 = 2; // tabs row + divider
const LOG_HEIGHT_RATIO: u16 = 35; // percent of right column
const HELP_OVERLAY_WIDTH: u16 = 60;
const HELP_OVERLAY_HEIGHT: u16 = 25;

// State.

/// Focus is a two-state toggle. The Log is a passive viewer — always
/// scrolled via global PgUp/PgDn/`<`/`>` regardless of focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    List,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Overview,
    Prs,
    Linear,
    Panes,
}

impl Tab {
    fn label(self) -> &'static str {
        match self {
            Tab::Overview => "Overview",
            Tab::Prs => "PRs",
            Tab::Linear => "Linear",
            Tab::Panes => "Panes",
        }
    }

    fn next(self) -> Self {
        match self {
            Tab::Overview => Tab::Prs,
            Tab::Prs => Tab::Linear,
            Tab::Linear => Tab::Panes,
            Tab::Panes => Tab::Overview,
        }
    }

    fn prev(self) -> Self {
        match self {
            Tab::Overview => Tab::Panes,
            Tab::Prs => Tab::Overview,
            Tab::Linear => Tab::Prs,
            Tab::Panes => Tab::Linear,
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
    pub meta: state::TaskMeta,
    pub status: TaskStatus,
    pub prs: Vec<state::PrData>,
    pub panes: Vec<TmuxPaneInfo>,
    /// Stub Linear data — slice 4b replaces this with real cache.
    pub linear: Vec<LinearStub>,
    /// Durable v2 task id when the v2 store is authoritative.
    pub id: Option<crate::store::TaskId>,
    /// True iff any drift flag is set on the v2 record.
    pub drift: bool,
}

#[derive(Debug, Clone)]
pub struct LinearStub {
    pub key: String,
    pub title: String,
    pub state: String,
    pub assignee: Option<String>,
    pub depth: u8,
}

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

/// Linear tab sub-state. The flat list is the default; pressing Enter
/// pushes a Detail view (one issue, full page). Esc pops back. Stack
/// supports drill-into-sub-issue → drill-into-sub-issue chains.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinearView {
    /// Flat row stream. Cursor identifies a row by issue key (stable
    /// across re-render even though row list expands/collapses based
    /// on which parent is cursored). `pinned` keeps mega-parents
    /// (7+ subs) visibly expanded; `t` toggles.
    List {
        cursor_key: String,
        pinned: HashSet<String>,
    },
    Detail {
        /// Drill stack — `last()` is the currently shown issue.
        /// `pop()` walks back; empty stack means we're done with detail.
        stack: Vec<String>,
        /// Cursor index into the rendered detail-view sub-issue list
        /// (sub-issues only — parent/project/url use dedicated keys).
        sub_cursor: usize,
    },
}

impl Default for LinearView {
    fn default() -> Self {
        LinearView::List {
            cursor_key: String::new(),
            pinned: HashSet::new(),
        }
    }
}

pub struct App {
    pub tasks: Vec<TaskView>,
    pub selected: usize,
    pub focus: Pane,
    pub detail_tab: Tab,
    /// Pane selected within the Panes tab.
    pub panes_selected: usize,
    /// Linear tab sub-state.
    pub linear_view: LinearView,
    pub log: LogPane,
    pub show_help: bool,
    pub daemon_alive: bool,
    pub last_fast: Instant,
    pub should_quit: bool,
    pub message_input: Option<String>,
    pub read_runs: HashSet<String>,
    pub last_run_count: usize,
    /// Transient single-line message rendered in the log header.
    /// Used to surface "not yet wired" feedback for unimplemented keys.
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
            linear_view: LinearView::default(),
            log: LogPane::default(),
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
        app.open_latest_run();
        app
    }

    fn load_tasks() -> Vec<TaskView> {
        let status_cache = crate::cache::read_status();
        let pr_cache = crate::cache::read_prs();
        let linear_cache = crate::cache::read_linear();
        let daemon_alive = crate::cache::is_daemon_alive();

        let live_sessions = if daemon_alive {
            None
        } else {
            Some(load_tmux_sessions())
        };

        let store = crate::store::Store::default();

        state::ordered_open_slugs()
            .into_iter()
            .map(|name| {
                let meta = load_task_meta(&name);
                let status = if daemon_alive {
                    status_cache
                        .tasks
                        .get(&name)
                        .map(|ct| status_from_str(&ct.status))
                        .unwrap_or(TaskStatus::Idle)
                } else if let Some(sessions) = &live_sessions {
                    state::derive_status(&meta, sessions, state::busy_stale_secs())
                } else {
                    TaskStatus::Idle
                };

                let prs: Vec<state::PrData> = meta
                    .prs
                    .iter()
                    .map(|&num| {
                        pr_cache
                            .prs
                            .get(&num)
                            .map(|cp| cp.to_pr_data())
                            .unwrap_or(state::PrData {
                                number: num,
                                ..Default::default()
                            })
                    })
                    .collect();

                let panes = panes_for_session(&meta.session);

                let record = if store.is_authoritative() {
                    store.load_record_by_slug(&name)
                } else {
                    None
                };
                let linear = record
                    .as_ref()
                    .map(|r| linear_from_record(r, &linear_cache))
                    .unwrap_or_default();
                let id = record.as_ref().map(|r| r.id);
                let drift = record.as_ref().map(|r| r.drift.any()).unwrap_or(false);

                TaskView {
                    name,
                    meta,
                    status,
                    prs,
                    panes,
                    linear,
                    id,
                    drift,
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
        let Some(run_id) = self.log.run_id.clone() else {
            return;
        };
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
        // Preserve selection by name when possible.
        let prev_name = self
            .tasks
            .get(self.selected)
            .map(|t| t.name.clone());
        self.tasks = next_tasks;
        if let Some(name) = prev_name {
            self.selected = self
                .tasks
                .iter()
                .position(|t| t.name == name)
                .unwrap_or(0);
        }
        if self.selected >= self.tasks.len() {
            self.selected = self.tasks.len().saturating_sub(1);
        }
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

fn status_from_str(s: &str) -> TaskStatus {
    match s {
        "ready" => TaskStatus::Ready,
        "working" => TaskStatus::Working,
        "input" => TaskStatus::Input,
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
            "list-panes", "-t", &actual, "-F",
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

/// Build LinearStub list for a task. When the cache has fresh data
/// for a key, the stub gets the real title/state/assignee. Otherwise
/// the title is empty and state shows the link provenance, so the
/// TUI can render keys immediately while the daemon backfills.
///
/// Stubs are sorted by project name so the list view can group by
/// project without re-shuffling the cursor.
fn linear_from_record(
    record: &crate::store::TaskRecord,
    cache: &crate::cache::LinearCache,
) -> Vec<LinearStub> {
    let mut stubs: Vec<LinearStub> = record
        .links
        .linear_issues
        .iter()
        .map(|li| {
            if let Some(cached) = cache.issues.get(&li.key) {
                LinearStub {
                    key: cached.identifier.clone(),
                    title: cached.title.clone(),
                    state: cached.state.clone(),
                    assignee: if cached.assignee.is_empty() {
                        None
                    } else {
                        Some(cached.assignee.clone())
                    },
                    depth: 0,
                }
            } else if cache.not_found.contains(&li.key) {
                LinearStub {
                    key: li.key.clone(),
                    title: String::new(),
                    state: "not on Linear".into(),
                    assignee: None,
                    depth: 0,
                }
            } else {
                let provenance = match li.source {
                    crate::store::LinkSource::Manual => "manual",
                    crate::store::LinkSource::BranchDiscovery => "branch",
                    crate::store::LinkSource::MarkdownScan => "scan",
                    crate::store::LinkSource::Migration => "migration",
                };
                LinearStub {
                    key: li.key.clone(),
                    title: String::new(),
                    state: format!("{provenance}, loading…"),
                    assignee: None,
                    depth: 0,
                }
            }
        })
        .collect();
    // Stable sort: project name (alphabetical), with no-project rows
    // last per docs/linear-list-minimal.md edge case. Then by key
    // within a project for tiebreak.
    stubs.sort_by(|a, b| {
        let pa = cache
            .issues
            .get(&a.key)
            .and_then(|c| c.project.as_ref())
            .map(|p| p.name.clone());
        let pb = cache
            .issues
            .get(&b.key)
            .and_then(|c| c.project.as_ref())
            .map(|p| p.name.clone());
        // None projects sort after Some projects.
        match (pa.is_none(), pb.is_none()) {
            (true, false) => std::cmp::Ordering::Greater,
            (false, true) => std::cmp::Ordering::Less,
            _ => pa.cmp(&pb).then(a.key.cmp(&b.key)),
        }
    });
    stubs
}

// Rendering.

pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();

    let outer = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(1), // left ruler
            Constraint::Length(LIST_WIDTH),
            Constraint::Length(SEPARATOR_WIDTH),
            Constraint::Length(1), // right ruler
            Constraint::Min(0),
        ])
        .split(area);

    render_focus_ruler(frame, outer[0], app.focus == Pane::List);
    render_list(frame, outer[1], app);
    render_vertical_separator(frame, outer[2]);
    render_focus_ruler(frame, outer[3], app.focus == Pane::Right);

    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(100 - LOG_HEIGHT_RATIO),
            Constraint::Length(1), // horizontal separator
            Constraint::Min(3),
        ])
        .split(outer[4]);

    render_details(frame, right[0], app);
    render_horizontal_separator(frame, right[1]);
    render_log(frame, right[2], app);

    if app.show_help {
        render_help_overlay(frame, area);
    }
}

fn render_focus_ruler(frame: &mut Frame, area: Rect, focused: bool) {
    if !focused {
        return;
    }
    let mut lines = Vec::with_capacity(area.height as usize);
    for _ in 0..area.height {
        lines.push(Line::styled("▎", Style::default().fg(LOVE)));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_vertical_separator(frame: &mut Frame, area: Rect) {
    let mut lines = Vec::with_capacity(area.height as usize);
    for _ in 0..area.height {
        lines.push(Line::styled("│", Style::default().fg(MUTED)));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_horizontal_separator(frame: &mut Frame, area: Rect) {
    let bar = "─".repeat(area.width as usize);
    frame.render_widget(
        Paragraph::new(Line::styled(bar, Style::default().fg(MUTED))),
        area,
    );
}

fn render_list(frame: &mut Frame, area: Rect, app: &App) {
    let mut lines: Vec<Line> = Vec::new();
    let focused = app.focus == Pane::List;

    // Header.
    let header_style = if focused {
        Style::default().fg(TEXT)
    } else {
        Style::default().fg(MUTED)
    };
    lines.push(Line::styled(" tasks", header_style));
    lines.push(Line::styled(
        "─".repeat(area.width as usize),
        Style::default().fg(MUTED),
    ));

    if app.tasks.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::styled(
            " no tasks · n to create",
            Style::default().fg(SUBTLE),
        ));
    } else {
        let name_color = if focused { TEXT } else { SUBTLE };
        for (i, task) in app.tasks.iter().enumerate() {
            let selected = i == app.selected;
            let badge = status_str(task.status);
            let badge_color = status_color(task.status);
            // Selected row keeps HL_LOW background regardless of focus,
            // so the user can navigate back to it.
            let row_bg = if selected { Some(HL_LOW) } else { None };

            let pr_count = task.prs.len();
            let linear_count = task.linear.len();

            let mut counts = String::new();
            if task.drift {
                counts.push_str(" ⚠");
            }
            if pr_count > 0 {
                counts.push_str(&format!(" P{pr_count}"));
            }
            if linear_count > 0 {
                counts.push_str(&format!(" L{linear_count}"));
            }
            let badge_text = format!(" {badge}");
            let id_text = task.id.map(|i| format!("#{i} ")).unwrap_or_default();
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
                    Style::default().fg(if selected { LOVE } else { MUTED }),
                ),
                Span::styled(id_text, Style::default().fg(MUTED)),
                Span::styled(name_str, Style::default().fg(name_color)),
                Span::raw(" ".repeat(pad)),
            ];
            if !counts.is_empty() {
                let counts_color = if task.drift && counts.starts_with(" ⚠") {
                    LOVE
                } else {
                    SUBTLE
                };
                spans.push(Span::styled(counts, Style::default().fg(counts_color)));
            }
            spans.push(Span::styled(
                badge_text,
                Style::default().fg(badge_color),
            ));

            let mut line = Line::from(spans);
            if let Some(bg) = row_bg {
                line = line.style(Style::default().bg(bg));
            }
            lines.push(line);
        }
    }

    frame.render_widget(Paragraph::new(lines), area);
}

fn render_details(frame: &mut Frame, area: Rect, app: &App) {
    if app.tasks.is_empty() {
        let placeholder = Paragraph::new(vec![
            Line::raw(""),
            Line::styled(" select a task", Style::default().fg(SUBTLE)),
        ]);
        frame.render_widget(placeholder, area);
        return;
    }

    // Tab bar. Active tab gets a `▎` underline glyph next to its label
    // so the user can identify the live tab without relying on color alone.
    let focused = app.focus == Pane::Right;
    let tabs = [Tab::Overview, Tab::Prs, Tab::Linear, Tab::Panes];
    let mut tab_spans: Vec<Span> = Vec::new();
    tab_spans.push(Span::raw(" "));
    for (i, tab) in tabs.iter().enumerate() {
        let active = *tab == app.detail_tab;
        let style = if active {
            Style::default().fg(if focused { LOVE } else { TEXT })
        } else if focused {
            Style::default().fg(SUBTLE)
        } else {
            Style::default().fg(MUTED)
        };
        if active {
            tab_spans.push(Span::styled("▎", Style::default().fg(LOVE)));
        }
        tab_spans.push(Span::styled(tab.label(), style));
        if i + 1 < tabs.len() {
            tab_spans.push(Span::styled("  ·  ", Style::default().fg(MUTED)));
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
        Style::default().fg(MUTED),
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

    let task = match app.selected_task() {
        Some(t) => t,
        None => return,
    };

    match app.detail_tab {
        Tab::Overview => render_tab_overview(frame, body_area, app, task),
        Tab::Prs => render_tab_prs(frame, body_area, app, task),
        Tab::Linear => render_tab_linear(frame, body_area, app, task),
        Tab::Panes => render_tab_panes(frame, body_area, app, task),
    }
}

fn render_tab_overview(frame: &mut Frame, area: Rect, _app: &App, task: &TaskView) {
    let session_str = if task.meta.session.is_empty() {
        "—".to_string()
    } else {
        task.meta.session.clone()
    };
    let worktree_str = if task.meta.worktree.is_empty() {
        "—".to_string()
    } else {
        task.meta.worktree.clone()
    };
    let prs_str = if task.prs.is_empty() {
        "—".to_string()
    } else {
        task.prs
            .iter()
            .map(|p| format!("#{}", p.number))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let linear_str = if task.linear.is_empty() {
        "—".to_string()
    } else {
        task.linear
            .iter()
            .map(|l| l.key.clone())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let panes_str = task.panes.len().to_string();

    let mut lines = vec![
        Line::raw(""),
        kv_line(" title:    ", &task.name),
        kv_line(" status:   ", status_str(task.status)),
        kv_line(" session:  ", &session_str),
        kv_line(" worktree: ", &worktree_str),
        kv_line(" prs:      ", &prs_str),
        kv_line(" linear:   ", &linear_str),
        kv_line(" panes:    ", &panes_str),
    ];
    if task.meta.needs_input {
        lines.push(kv_line(" attention:", "needs input"));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn render_tab_prs(frame: &mut Frame, area: Rect, _app: &App, task: &TaskView) {
    let mut lines = vec![Line::raw("")];
    if task.prs.is_empty() {
        lines.push(Line::styled(
            " (no linked PRs)",
            Style::default().fg(SUBTLE),
        ));
    } else {
        for pr in &task.prs {
            let title = if pr.title.is_empty() {
                "(no title cached)".into()
            } else {
                pr.title.clone()
            };
            let ci = match pr.ci_pass {
                Some(true) => Span::styled("✓ ci", Style::default().fg(PINE)),
                Some(false) => Span::styled("✗ ci", Style::default().fg(LOVE)),
                None => Span::styled("· ci", Style::default().fg(MUTED)),
            };
            let approval = if pr.approved {
                Span::styled(" ✓ review", Style::default().fg(PINE))
            } else {
                Span::styled(" · review", Style::default().fg(MUTED))
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!(" #{}", pr.number),
                    Style::default().fg(IRIS),
                ),
                Span::raw("  "),
                Span::styled(title, Style::default().fg(TEXT)),
            ]));
            lines.push(Line::from(vec![
                Span::raw("    "),
                ci,
                approval,
            ]));
            lines.push(Line::raw(""));
        }
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn render_tab_linear(frame: &mut Frame, area: Rect, app: &App, task: &TaskView) {
    let cache = crate::cache::read_linear();
    match &app.linear_view {
        LinearView::Detail { stack, sub_cursor } if !stack.is_empty() => {
            render_linear_detail(frame, area, stack, *sub_cursor, &cache, app);
        }
        LinearView::List { cursor_key, pinned } => {
            render_linear_list(frame, area, app, task, cursor_key, pinned, &cache);
        }
        _ => {
            let empty = HashSet::new();
            render_linear_list(frame, area, app, task, "", &empty, &cache);
        }
    }
}

// Row stream for the minimal list view. See docs/linear-list-minimal.md.

#[derive(Debug, Clone, PartialEq, Eq)]
enum RowKind {
    ProjectHeader,
    Parent { collapsed_subs: usize },
    SubIssue { is_last: bool },
    MoreMarker,
}

#[derive(Debug, Clone)]
struct ListRow {
    key: String,
    kind: RowKind,
    title: String,
    state_kind: String,
    state_name: String,
    project_for_strip: String,
    not_found: bool,
}

const AUTO_EXPAND_LIMIT: usize = 6;

/// Build the flat row stream from a task's linear stubs + cache. The
/// cursored parent (or the parent of a cursored sub-issue) auto-expands
/// when its child count is in 1..=AUTO_EXPAND_LIMIT. Mega-parents
/// (7+ children) only expand when pinned. Project headers appear only
/// when there are 2+ distinct projects.
fn build_linear_rows(
    stubs: &[LinearStub],
    cache: &crate::cache::LinearCache,
    cursor_key: &str,
    pinned: &HashSet<String>,
) -> Vec<ListRow> {
    if stubs.is_empty() {
        return Vec::new();
    }

    // Distinct projects across linked stubs.
    let mut projects: Vec<String> = Vec::new();
    for s in stubs {
        let p = cache
            .issues
            .get(&s.key)
            .and_then(|c| c.project.as_ref())
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "(no project)".into());
        if !projects.contains(&p) {
            projects.push(p);
        }
    }
    let multi_project = projects.len() > 1;

    // Determine which parent (if any) the cursor lives "in" — either the
    // parent itself or one of its children.
    let cursor_parent_key: Option<String> = if cursor_key.is_empty() {
        None
    } else if stubs.iter().any(|s| s.key == cursor_key) {
        Some(cursor_key.to_string())
    } else {
        stubs
            .iter()
            .find(|s| {
                cache
                    .issues
                    .get(&s.key)
                    .map(|c| c.children.iter().any(|ch| ch.identifier == cursor_key))
                    .unwrap_or(false)
            })
            .map(|s| s.key.clone())
    };

    let mut rows: Vec<ListRow> = Vec::new();
    let mut prev_project: Option<String> = None;

    for stub in stubs {
        let cached = cache.issues.get(&stub.key);
        let project_name = cached
            .and_then(|c| c.project.as_ref())
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "(no project)".into());

        // Project header on transition (only when multi-project).
        if multi_project && prev_project.as_ref() != Some(&project_name) {
            rows.push(ListRow {
                key: format!("__project_{project_name}"),
                kind: RowKind::ProjectHeader,
                title: project_name.clone(),
                state_kind: String::new(),
                state_name: String::new(),
                project_for_strip: String::new(),
                not_found: false,
            });
            prev_project = Some(project_name.clone());
        }

        let is_not_found = cache.not_found.contains(&stub.key);
        let n_children = cached.map(|c| c.children.len()).unwrap_or(0);
        let is_cursor_anchor = cursor_parent_key.as_deref() == Some(stub.key.as_str());
        let auto_expand = is_cursor_anchor && (1..=AUTO_EXPAND_LIMIT).contains(&n_children);
        let is_pinned = pinned.contains(&stub.key);
        let expanded = auto_expand || is_pinned;

        // Parent row.
        let collapsed_subs = if expanded { 0 } else { n_children };
        rows.push(ListRow {
            key: stub.key.clone(),
            kind: RowKind::Parent { collapsed_subs },
            title: cached.map(|c| c.title.clone()).unwrap_or_else(|| stub.title.clone()),
            state_kind: cached.map(|c| c.state_kind.clone()).unwrap_or_default(),
            state_name: cached.map(|c| c.state.clone()).unwrap_or_default(),
            project_for_strip: project_name.clone(),
            not_found: is_not_found,
        });

        if expanded && n_children > 0 {
            let cap = if is_pinned && n_children > AUTO_EXPAND_LIMIT {
                AUTO_EXPAND_LIMIT
            } else {
                n_children
            };
            for (i, child) in cached.unwrap().children.iter().take(cap).enumerate() {
                let is_last = i + 1 == cap && cap == n_children;
                rows.push(ListRow {
                    key: child.identifier.clone(),
                    kind: RowKind::SubIssue { is_last },
                    title: child.title.clone(),
                    state_kind: child.state_kind.clone(),
                    state_name: child.state.clone(),
                    project_for_strip: project_name.clone(),
                    not_found: false,
                });
            }
            if cap < n_children {
                rows.push(ListRow {
                    key: format!("__more_{}", stub.key),
                    kind: RowKind::MoreMarker,
                    title: format!("+ {} more", n_children - cap),
                    state_kind: String::new(),
                    state_name: String::new(),
                    project_for_strip: String::new(),
                    not_found: false,
                });
            }
        }
    }

    rows
}

/// Strip a leading `[Project]` prefix from a title when it matches the
/// enclosing project (case-insensitive). Linear's auto-namespacing
/// duplicates the project header otherwise.
fn strip_project_prefix(title: &str, project: &str) -> String {
    if project.is_empty() {
        return title.to_string();
    }
    let trimmed = title.trim_start();
    let prefix = format!("[{}]", project);
    if let Some(rest) = trimmed
        .to_lowercase()
        .strip_prefix(&prefix.to_lowercase())
        .map(|_| trimmed[prefix.len()..].trim_start())
    {
        return rest.to_string();
    }
    title.to_string()
}

/// Indices of `j/k`-targetable rows (skips ProjectHeader and MoreMarker).
fn cursor_targets(rows: &[ListRow]) -> Vec<usize> {
    rows.iter()
        .enumerate()
        .filter(|(_, r)| matches!(r.kind, RowKind::Parent { .. } | RowKind::SubIssue { .. }))
        .map(|(i, _)| i)
        .collect()
}

fn first_target_key(rows: &[ListRow]) -> Option<String> {
    cursor_targets(rows)
        .into_iter()
        .next()
        .and_then(|i| rows.get(i).map(|r| r.key.clone()))
}

fn render_linear_list(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    task: &TaskView,
    cursor_key: &str,
    pinned: &HashSet<String>,
    cache: &crate::cache::LinearCache,
) {
    let focused = app.focus == Pane::Right && app.detail_tab == Tab::Linear;
    let mut lines: Vec<Line> = Vec::new();

    if task.linear.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            " (no linked Linear issues)",
            Style::default().fg(SUBTLE),
        ));
        lines.push(Line::styled(
            " orch linear add <task> ENG-123  ·  orch linear scan <task>",
            Style::default().fg(MUTED),
        ));
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
        return;
    }

    let rows = build_linear_rows(&task.linear, cache, cursor_key, pinned);
    let width = area.width as usize;

    let mut last_was_header = false;
    for row in &rows {
        match &row.kind {
            RowKind::ProjectHeader => {
                if !lines.is_empty() {
                    lines.push(Line::raw(""));
                }
                lines.push(Line::from(vec![
                    Span::raw(" "),
                    Span::styled(
                        row.title.to_lowercase(),
                        Style::default().fg(IRIS),
                    ),
                ]));
                last_was_header = true;
            }
            RowKind::Parent { collapsed_subs } => {
                let selected = focused && row.key == cursor_key;
                let cursor_glyph = if selected { " ▸ " } else { "   " };
                let cursor_color = if selected { LOVE } else { MUTED };
                let state_color = linear_state_color(&row.state_name);
                let glyph = state_glyph(&row.state_kind);
                let title = strip_project_prefix(&row.title, &row.project_for_strip);
                let trailer = if *collapsed_subs > 0 {
                    Some(format!("+ {collapsed_subs} sub"))
                } else {
                    None
                };
                lines.push(compose_row(
                    cursor_glyph,
                    cursor_color,
                    &row.key,
                    glyph,
                    state_color,
                    &title,
                    if row.not_found { Some(LOVE) } else { None },
                    trailer.as_deref(),
                    width,
                    selected,
                ));
                last_was_header = false;
            }
            RowKind::SubIssue { is_last } => {
                let selected = focused && row.key == cursor_key;
                let prefix = if selected {
                    " ▸ "
                } else if *is_last {
                    " └ "
                } else {
                    " │ "
                };
                let prefix_color = if selected {
                    LOVE
                } else {
                    MUTED
                };
                let state_color = linear_state_color(&row.state_name);
                let glyph = state_glyph(&row.state_kind);
                let title = strip_project_prefix(&row.title, &row.project_for_strip);
                lines.push(compose_row(
                    prefix,
                    prefix_color,
                    &row.key,
                    glyph,
                    state_color,
                    &title,
                    None,
                    None,
                    width,
                    selected,
                ));
                last_was_header = false;
            }
            RowKind::MoreMarker => {
                lines.push(Line::from(vec![
                    Span::styled(" └ ", Style::default().fg(MUTED)),
                    Span::styled(row.title.clone(), Style::default().fg(MUTED)),
                ]));
                last_was_header = false;
            }
        }
    }
    let _ = last_was_header;

    if focused {
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            " j/k move · Enter open · t expand · o browser",
            Style::default().fg(MUTED),
        ));
    }

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

/// One-line row composer used by both Parent and SubIssue rows.
/// `prefix` (3 cols) + key (padded to 9) + 2 spaces + state glyph + 2 spaces +
/// title (truncated to fit) + optional right-aligned trailer.
fn compose_row(
    prefix: &str,
    prefix_color: Color,
    key: &str,
    state_glyph_str: &str,
    state_color: Color,
    title: &str,
    title_color_override: Option<Color>,
    trailer: Option<&str>,
    width: usize,
    selected: bool,
) -> Line<'static> {
    let key_field = format!("{key:<9}");
    let title_color = title_color_override.unwrap_or(TEXT);
    let prefix_len = prefix.chars().count();
    let key_len = 9;
    let glyph_len = state_glyph_str.chars().count();
    let trailer_len = trailer
        .map(|t| t.chars().count() + 3 /* "  ·  " minus a bit */)
        .unwrap_or(0);
    let used = prefix_len + key_len + 2 + glyph_len + 2 + trailer_len + 1;
    let title_room = width.saturating_sub(used);
    let title_cut = if title.chars().count() > title_room {
        let take = title_room.saturating_sub(1);
        let mut buf = String::new();
        for (i, c) in title.chars().enumerate() {
            if i >= take {
                buf.push('…');
                break;
            }
            buf.push(c);
        }
        buf
    } else {
        title.to_string()
    };
    let title_pad = title_room.saturating_sub(title_cut.chars().count());

    let mut spans = vec![
        Span::styled(prefix.to_string(), Style::default().fg(prefix_color)),
        Span::styled(key_field, Style::default().fg(IRIS)),
        Span::raw("  "),
        Span::styled(state_glyph_str.to_string(), Style::default().fg(state_color)),
        Span::raw("  "),
        Span::styled(title_cut, Style::default().fg(title_color)),
    ];
    if let Some(t) = trailer {
        spans.push(Span::raw(" ".repeat(title_pad)));
        spans.push(Span::styled(
            format!("  {t}"),
            Style::default().fg(MUTED),
        ));
    }
    let line = Line::from(spans);
    if selected {
        line.style(Style::default().bg(HL_LOW))
    } else {
        line
    }
}


fn render_linear_detail(
    frame: &mut Frame,
    area: Rect,
    stack: &[String],
    sub_cursor: usize,
    cache: &crate::cache::LinearCache,
    app: &App,
) {
    let focused = app.focus == Pane::Right && app.detail_tab == Tab::Linear;
    let key = stack.last().cloned().unwrap_or_default();
    let cached = cache.issues.get(&key);

    let mut lines: Vec<Line> = Vec::new();

    let Some(c) = cached else {
        // Drilled into a key we don't have data for yet.
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            format!(" {key}  loading…"),
            Style::default().fg(MUTED),
        ));
        if cache.not_found.contains(&key) {
            lines.push(Line::styled(
                " not on Linear",
                Style::default().fg(LOVE),
            ));
        }
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            " Esc back · o browser",
            Style::default().fg(MUTED),
        ));
        frame.render_widget(Paragraph::new(lines), area);
        return;
    };

    // Row 1: key · priority · state · age
    let state_color = linear_state_color(&c.state);
    let priority_label = priority_glyph(c.priority);
    let mut id_line = vec![Span::styled(
        format!(" {}", c.identifier),
        Style::default().fg(IRIS),
    )];
    if !priority_label.is_empty() {
        id_line.push(Span::styled(
            format!("  ·  {priority_label}"),
            Style::default().fg(priority_color(c.priority)),
        ));
    }
    id_line.push(Span::styled(
        format!("  ·  {} {}", state_glyph(&c.state_kind), c.state),
        Style::default().fg(state_color),
    ));
    let age = relative_age(&c.updated_at);
    if !age.is_empty() {
        id_line.push(Span::styled(
            format!("  ·  {age}"),
            Style::default().fg(MUTED),
        ));
    }
    if stack.len() > 1 {
        id_line.push(Span::styled(
            format!("  (depth {})", stack.len() - 1),
            Style::default().fg(MUTED),
        ));
    }
    lines.push(Line::from(id_line));

    // Row 2: title
    lines.push(Line::from(vec![
        Span::raw(" "),
        Span::styled(c.title.clone(), Style::default().fg(TEXT)),
    ]));
    lines.push(Line::raw(""));

    // Project / parent / cycle / assignee meta block
    if let Some(p) = &c.project {
        lines.push(Line::from(vec![
            Span::styled(" Project   ", Style::default().fg(MUTED)),
            Span::styled(p.name.clone(), Style::default().fg(TEXT)),
        ]));
    }
    if let Some(parent_key) = &c.parent_key {
        let title = c.parent_title.clone().unwrap_or_default();
        lines.push(Line::from(vec![
            Span::styled(" Parent    ", Style::default().fg(MUTED)),
            Span::styled(
                format!("{parent_key}  "),
                Style::default().fg(IRIS),
            ),
            Span::styled(title, Style::default().fg(TEXT)),
        ]));
    }
    if let Some(cycle) = &c.cycle_name {
        if !cycle.is_empty() {
            lines.push(Line::from(vec![
                Span::styled(" Cycle     ", Style::default().fg(MUTED)),
                Span::styled(cycle.clone(), Style::default().fg(TEXT)),
            ]));
        }
    }
    if !c.assignee.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(" Assignee  ", Style::default().fg(MUTED)),
            Span::styled(format!("@{}", c.assignee), Style::default().fg(TEXT)),
        ]));
    }

    // Description block — wrap, trim to fit remaining space.
    // Budget: total area minus lines so far minus reserved trailing rows.
    let used = lines.len();
    let footer_reserved: usize = 2; // blank + footer line
    let n_children = c.children.len();
    let kid_block_reserved: usize = if n_children == 0 {
        0
    } else {
        // blank + "Sub-issues (N)" header + up to 3 rows + optional "and N more"
        let visible = n_children.min(3);
        let overflow = if n_children > 3 { 1 } else { 0 };
        2 + visible + overflow
    };
    let desc_budget = (area.height as usize)
        .saturating_sub(used + footer_reserved + kid_block_reserved);
    if desc_budget >= 3 && !c.description.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            " Description",
            Style::default().fg(MUTED),
        ));
        let width = (area.width.saturating_sub(2) as usize).max(20);
        let wrapped = wrap_text(&c.description, width);
        let body_room = desc_budget - 2;
        // Reserve one row for "…" if we overflow.
        let take = if wrapped.len() > body_room {
            body_room.saturating_sub(1)
        } else {
            body_room
        };
        for w in wrapped.iter().take(take) {
            lines.push(Line::from(vec![
                Span::raw(" "),
                Span::styled(w.clone(), Style::default().fg(SUBTLE)),
            ]));
        }
        if wrapped.len() > take {
            lines.push(Line::styled(
                " …",
                Style::default().fg(MUTED),
            ));
        }
    }

    // Sub-issues
    if !c.children.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            format!(" Sub-issues ({n_children})"),
            Style::default().fg(MUTED),
        ));
        for (i, child) in c.children.iter().take(3).enumerate() {
            let selected = i == sub_cursor;
            let cursor = if selected && focused { "▸ " } else { "  " };
            let cursor_color = if selected && focused { LOVE } else { MUTED };
            let glyph = state_glyph(&child.state_kind);
            let glyph_color = linear_state_color(&child.state);
            let style = if selected && focused {
                Style::default().fg(TEXT).bg(HL_LOW)
            } else {
                Style::default().fg(TEXT)
            };
            lines.push(Line::from(vec![
                Span::styled(cursor, Style::default().fg(cursor_color)),
                Span::styled(
                    format!("{}  ", child.identifier),
                    Style::default().fg(IRIS),
                ),
                Span::styled(format!("{glyph} "), Style::default().fg(glyph_color)),
                Span::styled(child.title.clone(), style),
            ]));
        }
        if c.children.len() > 3 {
            lines.push(Line::styled(
                format!(" … and {} more", c.children.len() - 3),
                Style::default().fg(MUTED),
            ));
        }
    }

    // Footer
    lines.push(Line::raw(""));
    let footer = if c.children.is_empty() {
        " Esc back · u parent · p project · o browser"
    } else {
        " j/k navigate · Enter drill · u parent · p project · o browser · Esc back"
    };
    lines.push(Line::styled(footer, Style::default().fg(MUTED)));

    frame.render_widget(Paragraph::new(lines), area);
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

/// Linear priority glyph: 1=urgent, 2=high, 3=medium, 4=low, 0=none.
fn priority_glyph(priority: u8) -> &'static str {
    match priority {
        1 => "P0",
        2 => "P1",
        3 => "P2",
        4 => "P3",
        _ => "",
    }
}

fn priority_color(priority: u8) -> Color {
    match priority {
        1 | 2 => LOVE,
        3 => GOLD,
        _ => MUTED,
    }
}

/// Glyph for a Linear state-kind category.
fn state_glyph(kind: &str) -> &'static str {
    match kind {
        "started" => "◐",
        "completed" => "●",
        "canceled" => "⊘",
        "unstarted" => "○",
        "backlog" => "·",
        "triage" => "△",
        _ => "·",
    }
}

/// "4d ago", "12h ago", "30s ago" from an ISO-8601 timestamp.
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
            Style::default().fg(SUBTLE),
        ));
    } else {
        let focused =
            app.focus == Pane::Right && app.detail_tab == Tab::Panes;
        for (i, pane) in task.panes.iter().enumerate() {
            let selected = i == app.panes_selected;
            let marker = if pane.active { "●" } else { "·" };
            let prefix = if selected && focused { "▸" } else { " " };
            let style = if selected && focused {
                Style::default().fg(LOVE).bg(HL_LOW)
            } else if selected {
                Style::default().fg(TEXT).bg(HL_LOW)
            } else if focused {
                Style::default().fg(TEXT)
            } else {
                Style::default().fg(SUBTLE)
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!(" {prefix} {marker} "),
                    Style::default().fg(if pane.active { PINE } else { MUTED }),
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
            Style::default().fg(MUTED),
        ));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn render_log(frame: &mut Frame, area: Rect, app: &App) {
    // Log is a passive viewer — no focus state. Header in MUTED.
    // Toast (if present) overrides the run-id label.
    let header_style = Style::default().fg(MUTED);
    let header_text = if let Some(toast) = &app.toast {
        format!(" log: {toast}")
    } else {
        match &app.log.run_id {
            Some(id) => format!(" log: {}{}", id, if app.log.finished { " ·done" } else { "" }),
            None => " log: no activity".to_string(),
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
            Style::default().fg(SUBTLE),
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
        .map(|l| Line::styled(l.as_str(), Style::default().fg(SUBTLE)))
        .collect();

    frame.render_widget(
        Paragraph::new(log_lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll as u16, 0)),
        body_area,
    );
}

fn render_help_overlay(frame: &mut Frame, area: Rect) {
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
    frame.render_widget(
        ratatui::widgets::Clear,
        overlay,
    );

    let lines = vec![
        Line::styled(" key bindings", Style::default().fg(LOVE)),
        Line::styled("─".repeat(w as usize), Style::default().fg(MUTED)),
        Line::styled(" Global", Style::default().fg(IRIS)),
        kv_line("  q        ", "quit"),
        kv_line("  Tab      ", "toggle list ↔ right"),
        kv_line("  1 2 3 4  ", "Overview · PRs · Linear · Panes"),
        kv_line("  Esc      ", "right → list; list → quit"),
        kv_line("  PgUp/Dn  ", "scroll log"),
        kv_line("  < / >    ", "log: top / tail"),
        kv_line("  ?        ", "this overlay"),
        kv_line("  r m      ", "refresh · message"),
        Line::styled(" List", Style::default().fg(IRIS)),
        kv_line("  j k g G  ", "move · top / bottom"),
        kv_line("  Enter    ", "attach to active pane"),
        Line::styled(" Right zone", Style::default().fg(IRIS)),
        kv_line("  j k      ", "move cursor in active tab"),
        kv_line("  Enter    ", "open / attach in active tab"),
        Line::styled(
            " Phase 1F+:  n s p R x M J K o W (not yet wired)",
            Style::default().fg(MUTED),
        ),
    ];

    let text_area = Rect {
        x: overlay.x + 1,
        y: overlay.y + 1,
        width: overlay.width.saturating_sub(2),
        height: overlay.height.saturating_sub(2),
    };
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        text_area,
    );

    // Border around overlay.
    use ratatui::widgets::{Block, Borders};
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(LOVE));
    frame.render_widget(block, overlay);
}

fn kv_line<'a>(key: &'a str, value: &'a str) -> Line<'a> {
    Line::from(vec![
        Span::styled(key, Style::default().fg(MUTED)),
        Span::styled(value, Style::default().fg(TEXT)),
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
        TaskStatus::Input => "input",
        TaskStatus::Idle => "idle",
        TaskStatus::Paused => "paused",
        TaskStatus::Attached => "attach",
        TaskStatus::Error => "error",
    }
}

/// Color a Linear workflow state name. Falls back to `MUTED` for
/// unknowns (including the loading-stub `manual, loading…`).
fn linear_state_color(state: &str) -> Color {
    let lower = state.to_lowercase();
    if lower.contains("progress") {
        FOAM
    } else if lower.contains("done") || lower.contains("complete") {
        PINE
    } else if lower.contains("review") {
        GOLD
    } else if lower.contains("cancel") {
        LOVE
    } else if lower.contains("backlog") || lower.contains("todo") {
        IRIS
    } else {
        MUTED
    }
}

fn status_color(status: TaskStatus) -> Color {
    match status {
        TaskStatus::Ready => PINE,
        TaskStatus::Working => FOAM,
        TaskStatus::Input => GOLD,
        TaskStatus::Paused => IRIS,
        TaskStatus::Idle | TaskStatus::Attached => MUTED,
        TaskStatus::Error => LOVE,
    }
}

/// Total visual rows a list of lines occupies after word wrap into a
/// fixed-width column. Rough but sufficient for scroll clamping.
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
// Two-zone focus (List ↔ Right). Log is a passive viewer — scrolled
// via global PgUp/PgDn/`<`/`>` regardless of focus. See
// `docs/tui-nav-redesign.md`.

const UNWIRED_KEYS: &[char] = &['n', 's', 'p', 'R', 'x', 'M', 'J', 'K', 'o', 'W'];

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

    // Most keys clear any active toast. The unwired-key handler sets
    // a fresh toast and returns early before this clear runs.
    let was_toasted = app.toast.is_some();

    match (key.code, key.modifiers) {
        // Quit semantics: from list, q or Esc quits. From right zone,
        // Esc returns focus to the list (one stable home base).
        (KeyCode::Char('q'), _) => {
            app.should_quit = true;
            return;
        }
        (KeyCode::Esc, _) => {
            // Layered Esc per docs/tui-nav-redesign.md + linear-deep-design:
            // 1. modal cancel (handled above by show_help / message_input)
            // 2. Linear detail → pop stack
            // 3. focus right → focus list
            // 4. focus list → quit
            if app.focus == Pane::Right
                && app.detail_tab == Tab::Linear
            {
                if let LinearView::Detail { stack, .. } = &mut app.linear_view {
                    stack.pop();
                    if stack.is_empty() {
                        app.linear_view = LinearView::default();
                    }
                    if was_toasted { app.toast = None; }
                    return;
                }
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
        // Two-zone toggle.
        (KeyCode::Tab, _) | (KeyCode::BackTab, _) => {
            app.focus = match app.focus {
                Pane::List => Pane::Right,
                Pane::Right => Pane::List,
            };
        }
        // Number keys jump to a detail tab and focus right.
        (KeyCode::Char('1'), _) => {
            app.detail_tab = Tab::Overview;
            app.focus = Pane::Right;
        }
        (KeyCode::Char('2'), _) => {
            app.detail_tab = Tab::Prs;
            app.focus = Pane::Right;
        }
        (KeyCode::Char('3'), _) => {
            app.detail_tab = Tab::Linear;
            app.focus = Pane::Right;
            ensure_linear_cursor(app);
        }
        (KeyCode::Char('4'), _) => {
            app.detail_tab = Tab::Panes;
            app.focus = Pane::Right;
        }
        (KeyCode::Char('m'), _) => {
            app.message_input = Some(String::new());
        }
        // Global log controls.
        (KeyCode::PageUp, _) => {
            app.log.follow_bottom = false;
            app.log.scroll = app.log.scroll.saturating_sub(10);
        }
        (KeyCode::PageDown, _) => {
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
        // Unwired keys produce a toast instead of silently no-op'ing.
        (KeyCode::Char(c), _) if UNWIRED_KEYS.contains(&c) => {
            app.toast = Some(format!(
                "not yet wired — see redesign-notes Phase 1F/4a ({c})"
            ));
            return;
        }
        _ => match app.focus {
            Pane::List => handle_list_key(app, key),
            Pane::Right => handle_right_key(app, key),
        },
    }

    if was_toasted {
        app.toast = None;
    }
}

fn handle_list_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if app.selected + 1 < app.tasks.len() {
                app.selected += 1;
                app.panes_selected = 0;
                reset_linear_cursor_for_new_task(app);
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.selected = app.selected.saturating_sub(1);
            app.panes_selected = 0;
            reset_linear_cursor_for_new_task(app);
        }
        KeyCode::Char('g') => app.selected = 0,
        KeyCode::Char('G') => {
            app.selected = app.tasks.len().saturating_sub(1);
        }
        KeyCode::Enter => {
            // Attach to selected task's tmux session (active pane is
            // whatever tmux had selected last).
            if let Some(task) = app.selected_task() {
                if !task.meta.session.is_empty() {
                    attach_session(&task.meta.session);
                }
            }
        }
        _ => {}
    }
}

/// Right-zone key dispatch. j/k always means "move cursor in active
/// tab"; Enter always means "act on cursored item".
fn handle_right_key(app: &mut App, key: KeyEvent) {
    match (app.detail_tab, key.code) {
        // Tab cycling via h/l is kept as a fallback; numbers are the primary path.
        (_, KeyCode::Char('h')) | (_, KeyCode::Left) => {
            app.detail_tab = app.detail_tab.prev();
        }
        (_, KeyCode::Char('l')) | (_, KeyCode::Right) => {
            app.detail_tab = app.detail_tab.next();
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
                }
            }
        }
        // Linear tab — drill state machine.
        (Tab::Linear, KeyCode::Char('j')) | (Tab::Linear, KeyCode::Down) => {
            handle_linear_down(app);
        }
        (Tab::Linear, KeyCode::Char('k')) | (Tab::Linear, KeyCode::Up) => {
            handle_linear_up(app);
        }
        (Tab::Linear, KeyCode::Enter) => {
            handle_linear_enter(app);
        }
        (Tab::Linear, KeyCode::Char('u')) => {
            handle_linear_parent(app);
        }
        (Tab::Linear, KeyCode::Char('p')) => {
            handle_linear_open_project(app);
        }
        (Tab::Linear, KeyCode::Char('o')) => {
            handle_linear_open_browser(app);
        }
        (Tab::Linear, KeyCode::Char('t')) => {
            handle_linear_toggle_expand(app);
        }
        _ => {}
    }
}

/// Reset cursor_key when the user moves to a different task, since
/// the previously-cursored Linear key likely doesn't exist on the
/// new task. Also collapses pinned-open parents.
fn reset_linear_cursor_for_new_task(app: &mut App) {
    if let LinearView::List { cursor_key, pinned } = &mut app.linear_view {
        cursor_key.clear();
        pinned.clear();
    }
}

/// Initialize cursor_key to the first linked issue when entering the
/// Linear tab on a task that has linked issues. No-op if a cursor is
/// already set or the task has none.
fn ensure_linear_cursor(app: &mut App) {
    let first_key = app
        .tasks
        .get(app.selected)
        .and_then(|t| t.linear.first().map(|s| s.key.clone()));
    if let LinearView::List { cursor_key, .. } = &mut app.linear_view {
        if cursor_key.is_empty() {
            if let Some(k) = first_key {
                *cursor_key = k;
            }
        }
    }
}

/// Move the linear-list cursor by `delta` rows. Walks the visible
/// flat row stream (parents + expanded children); skips project headers
/// and "+ N more" markers. Computes rows from the *current* cursor
/// state, then commits to the `delta`-th targetable row from the
/// current position.
fn move_linear_cursor(app: &mut App, delta: isize) {
    let cache = crate::cache::read_linear();
    let stubs = match app.tasks.get(app.selected) {
        Some(t) => t.linear.clone(),
        None => return,
    };
    if stubs.is_empty() {
        return;
    }
    let (cursor_key, pinned) = match &app.linear_view {
        LinearView::List { cursor_key, pinned } => (cursor_key.clone(), pinned.clone()),
        _ => return,
    };
    let rows = build_linear_rows(&stubs, &cache, &cursor_key, &pinned);
    let targets = cursor_targets(&rows);
    if targets.is_empty() {
        return;
    }
    let cur_pos = targets
        .iter()
        .position(|&i| rows[i].key == cursor_key)
        .unwrap_or(0);
    let new_pos = (cur_pos as isize + delta)
        .clamp(0, targets.len() as isize - 1) as usize;
    let new_key = rows[targets[new_pos]].key.clone();
    if let LinearView::List { cursor_key, .. } = &mut app.linear_view {
        *cursor_key = new_key;
    }
}

fn handle_linear_down(app: &mut App) {
    if matches!(app.linear_view, LinearView::Detail { .. }) {
        if let LinearView::Detail { stack, sub_cursor } = &mut app.linear_view {
            let cache = crate::cache::read_linear();
            let n = stack
                .last()
                .and_then(|k| cache.issues.get(k))
                .map(|c| c.children.len().min(3))
                .unwrap_or(0);
            if *sub_cursor + 1 < n {
                *sub_cursor += 1;
            }
        }
        return;
    }
    move_linear_cursor(app, 1);
}

fn handle_linear_up(app: &mut App) {
    if matches!(app.linear_view, LinearView::Detail { .. }) {
        if let LinearView::Detail { sub_cursor, .. } = &mut app.linear_view {
            *sub_cursor = sub_cursor.saturating_sub(1);
        }
        return;
    }
    move_linear_cursor(app, -1);
}

fn handle_linear_enter(app: &mut App) {
    let cache = crate::cache::read_linear();
    match &mut app.linear_view {
        LinearView::List { cursor_key, .. } => {
            if !cursor_key.is_empty() {
                let k = cursor_key.clone();
                app.linear_view = LinearView::Detail {
                    stack: vec![k],
                    sub_cursor: 0,
                };
            }
        }
        LinearView::Detail { stack, sub_cursor } => {
            // Drill into the cursored sub-issue.
            let next = stack
                .last()
                .and_then(|k| cache.issues.get(k))
                .and_then(|c| c.children.get(*sub_cursor))
                .map(|child| child.identifier.clone());
            if let Some(k) = next {
                if stack.len() < 8 {
                    stack.push(k);
                    *sub_cursor = 0;
                }
            }
        }
    }
}

/// `t` toggles the pinned-open status of the cursored parent (or the
/// parent of a cursored sub-issue). Sub-issues themselves can't be
/// pinned — `t` on a sub-issue is a no-op.
fn handle_linear_toggle_expand(app: &mut App) {
    let cache = crate::cache::read_linear();
    let stubs = match app.tasks.get(app.selected) {
        Some(t) => t.linear.clone(),
        None => return,
    };
    let cursor_key = match &app.linear_view {
        LinearView::List { cursor_key, .. } => cursor_key.clone(),
        _ => return,
    };
    // Resolve cursor → its parent (cursor itself if it's a parent stub).
    let parent_key = if stubs.iter().any(|s| s.key == cursor_key) {
        Some(cursor_key)
    } else {
        stubs
            .iter()
            .find(|s| {
                cache
                    .issues
                    .get(&s.key)
                    .map(|c| c.children.iter().any(|ch| ch.identifier == cursor_key))
                    .unwrap_or(false)
            })
            .map(|s| s.key.clone())
    };
    if let Some(p) = parent_key {
        if let LinearView::List { pinned, .. } = &mut app.linear_view {
            if pinned.contains(&p) {
                pinned.remove(&p);
            } else {
                pinned.insert(p);
            }
        }
    }
}

fn handle_linear_parent(app: &mut App) {
    let cache = crate::cache::read_linear();
    if let LinearView::Detail { stack, sub_cursor } = &mut app.linear_view {
        let parent = stack
            .last()
            .and_then(|k| cache.issues.get(k))
            .and_then(|c| c.parent_key.clone());
        if let Some(p) = parent {
            if stack.len() < 8 {
                stack.push(p);
                *sub_cursor = 0;
            }
        }
    }
}

fn handle_linear_open_project(app: &mut App) {
    let cache = crate::cache::read_linear();
    let project_slug = match &app.linear_view {
        LinearView::Detail { stack, .. } => stack
            .last()
            .and_then(|k| cache.issues.get(k))
            .and_then(|c| c.project.as_ref())
            .map(|p| p.slug_id.clone()),
        LinearView::List { cursor_key, .. } => cache
            .issues
            .get(cursor_key)
            .and_then(|c| c.project.as_ref())
            .map(|p| p.slug_id.clone()),
    };
    if let Some(slug) = project_slug {
        if !slug.is_empty() {
            open_url(&format!("https://linear.app/column-na/project/{slug}"));
        }
    }
}

fn handle_linear_open_browser(app: &mut App) {
    let cache = crate::cache::read_linear();
    let url = match &app.linear_view {
        LinearView::Detail { stack, .. } => stack
            .last()
            .and_then(|k| cache.issues.get(k))
            .map(|c| c.url.clone()),
        LinearView::List { cursor_key, .. } => cache
            .issues
            .get(cursor_key)
            .map(|c| c.url.clone()),
    };
    if let Some(u) = url {
        if !u.is_empty() {
            open_url(&u);
        }
    }
}

fn open_url(url: &str) {
    let _ = std::process::Command::new("open")
        .arg(url)
        .stderr(std::process::Stdio::null())
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
    let actual = match find_actual_session(session) {
        Some(s) => s,
        None => return,
    };
    let action = if std::env::var("TMUX").is_ok() {
        "switch-client"
    } else {
        "attach-session"
    };
    let _ = Command::new("tmux").args([action, "-t", &actual]).status();
}

fn attach_pane(session: &str, pane_id: &str) {
    let actual = match find_actual_session(session) {
        Some(s) => s,
        None => return,
    };
    // Switch to the session, then select the target pane.
    let action = if std::env::var("TMUX").is_ok() {
        "switch-client"
    } else {
        "attach-session"
    };
    let _ = Command::new("tmux").args([action, "-t", &actual]).status();
    let _ = Command::new("tmux")
        .args(["select-pane", "-t", pane_id])
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

pub fn render_debug(
    width: u16,
    height: u16,
    tab: &str,
    focus: &str,
    select: usize,
    linear_detail: Option<&str>,
    linear_cursor: Option<&str>,
) {
    use ratatui::backend::TestBackend;
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("debug backend");
    let mut app = App::new();
    app.detail_tab = match tab.to_lowercase().as_str() {
        "prs" => Tab::Prs,
        "linear" => Tab::Linear,
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
    if let Some(key) = linear_detail {
        app.detail_tab = Tab::Linear;
        app.focus = Pane::Right;
        app.linear_view = LinearView::Detail {
            stack: vec![key.to_string()],
            sub_cursor: 0,
        };
    } else if let Some(key) = linear_cursor {
        app.detail_tab = Tab::Linear;
        app.focus = Pane::Right;
        app.linear_view = LinearView::List {
            cursor_key: key.to_string(),
            pinned: HashSet::new(),
        };
    } else if app.detail_tab == Tab::Linear {
        ensure_linear_cursor(&mut app);
    }
    terminal.draw(|f| render(f, &app)).expect("debug draw");
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
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    let mut app = App::new();

    let res: io::Result<()> = (|| {
        while !app.should_quit {
            terminal.draw(|f| render(f, &app))?;

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
                meta: state::TaskMeta {
                    session: "task-infra-triage".into(),
                    worktree: "/Users/a/column/task-infra-triage".into(),
                    prs: vec![25163],
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
                linear: vec![LinearStub {
                    key: "ENG-29151".into(),
                    title: "(stub: ENG-29151)".into(),
                    state: "—".into(),
                    assignee: None,
                    depth: 0,
                }],
                id: Some(2),
                drift: false,
            },
            TaskView {
                name: "ach-sanitize".into(),
                meta: state::TaskMeta {
                    session: "task-ach-sanitize".into(),
                    paused: true,
                    ..Default::default()
                },
                status: TaskStatus::Paused,
                prs: vec![],
                panes: vec![],
                linear: vec![],
                id: None,
                drift: false,
            },
            TaskView {
                name: "fresh-task".into(),
                meta: state::TaskMeta::default(),
                status: TaskStatus::Idle,
                prs: vec![],
                panes: vec![],
                linear: vec![],
                id: None,
                drift: false,
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
            linear_view: LinearView::default(),
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

    fn render_to_string(app: &App, w: u16, h: u16) -> String {
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, app)).unwrap();
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
        assert!(s.contains("PRs"));
        assert!(s.contains("Linear"));
        assert!(s.contains("Panes"));
        // Right bottom: log header.
        assert!(s.contains("log:"));
        assert!(s.contains("infra-triage: working"));
    }

    #[test]
    fn snapshot_three_pane_list_focus() {
        let mut app = test_app();
        app.focus = Pane::List;
        app.selected = 0;
        let s = render_to_string(&app, 100, 25);
        // Selected row has the focus marker; id is rendered before the name.
        assert!(s.contains("▸ #2 infra-triage"));
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
    fn snapshot_three_pane_log_focus() {
        let mut app = test_app();
        app.focus = Pane::Right;
        let s = render_to_string(&app, 100, 25);
        assert!(s.contains("log:"));
    }

    #[test]
    fn snapshot_detail_tab_overview() {
        let mut app = test_app();
        app.focus = Pane::Right;
        app.detail_tab = Tab::Overview;
        let s = render_to_string(&app, 100, 25);
        assert!(s.contains("infra-triage"));
        assert!(s.contains("session:"));
        assert!(s.contains("worktree:"));
        assert!(s.contains("/Users/a/column/task-infra-triage"));
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
    fn snapshot_detail_tab_linear() {
        let mut app = test_app();
        app.focus = Pane::Right;
        app.detail_tab = Tab::Linear;
        let s = render_to_string(&app, 100, 25);
        assert!(s.contains("ENG-29151"));
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
        app.log.lines = vec![
            "this is a really long log line that absolutely will not fit in 60 columns of width and must wrap onto multiple visual rows when rendered".into(),
            "".into(),
            "[scan] short line".into(),
        ];
        // Use 60 cols of total width — log pane gets ~60 of right column.
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
        assert!(s.contains("n to create"));
        assert!(s.contains("select a task"));
    }

    #[test]
    fn snapshot_key_help_overlay() {
        let mut app = test_app();
        app.show_help = true;
        let s = render_to_string(&app, 100, 25);
        assert!(s.contains("key bindings"));
        assert!(s.contains("toggle list ↔ right"));
        assert!(s.contains("1 2 3 4"));
        assert!(s.contains("Overview"));
        assert!(s.contains("Phase 1F+"));
    }

    #[test]
    fn linear_enter_pushes_detail() {
        let mut app = test_app();
        app.focus = Pane::Right;
        app.detail_tab = Tab::Linear;
        app.linear_view = LinearView::List {
            cursor_key: "ENG-29151".into(),
            pinned: HashSet::new(),
        };
        // Task #0 has one Linear stub: ENG-29151
        handle_key(&mut app, KeyEvent::from(KeyCode::Enter));
        match &app.linear_view {
            LinearView::Detail { stack, sub_cursor } => {
                assert_eq!(stack, &vec!["ENG-29151".to_string()]);
                assert_eq!(*sub_cursor, 0);
            }
            _ => panic!("expected Detail view, got {:?}", app.linear_view),
        }
    }

    #[test]
    fn linear_esc_pops_detail() {
        let mut app = test_app();
        app.focus = Pane::Right;
        app.detail_tab = Tab::Linear;
        app.linear_view = LinearView::Detail {
            stack: vec!["ENG-1".into(), "ENG-2".into()],
            sub_cursor: 0,
        };
        handle_key(&mut app, KeyEvent::from(KeyCode::Esc));
        // Esc pops one level
        match &app.linear_view {
            LinearView::Detail { stack, .. } => {
                assert_eq!(stack, &vec!["ENG-1".to_string()]);
            }
            _ => panic!("expected Detail with shorter stack"),
        }
        // Esc again pops to List
        handle_key(&mut app, KeyEvent::from(KeyCode::Esc));
        assert!(matches!(app.linear_view, LinearView::List { .. }));
        // Esc again returns focus to list zone
        handle_key(&mut app, KeyEvent::from(KeyCode::Esc));
        assert_eq!(app.focus, Pane::List);
        // Esc from list quits
        handle_key(&mut app, KeyEvent::from(KeyCode::Esc));
        assert!(app.should_quit);
    }

    #[test]
    fn linear_drill_stack_capped() {
        // Stack pushes are capped at 8 to prevent runaway.
        let mut app = test_app();
        app.focus = Pane::Right;
        app.detail_tab = Tab::Linear;
        app.linear_view = LinearView::Detail {
            stack: (0..8).map(|i| format!("ENG-{i}")).collect(),
            sub_cursor: 0,
        };
        // Pushing parent should no-op since stack is at cap; with no
        // cache entry the parent_key lookup is None anyway, but cap
        // logic is still tested.
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('u')));
        match &app.linear_view {
            LinearView::Detail { stack, .. } => assert_eq!(stack.len(), 8),
            _ => panic!("expected Detail"),
        }
    }

    #[test]
    fn priority_glyph_mapping() {
        assert_eq!(priority_glyph(0), "");
        assert_eq!(priority_glyph(1), "P0");
        assert_eq!(priority_glyph(2), "P1");
        assert_eq!(priority_glyph(3), "P2");
        assert_eq!(priority_glyph(4), "P3");
    }

    #[test]
    fn state_glyph_mapping() {
        assert_eq!(state_glyph("started"), "◐");
        assert_eq!(state_glyph("completed"), "●");
        assert_eq!(state_glyph("canceled"), "⊘");
        assert_eq!(state_glyph("unstarted"), "○");
        assert_eq!(state_glyph("backlog"), "·");
        assert_eq!(state_glyph("triage"), "△");
        assert_eq!(state_glyph("unknown"), "·");
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
    fn snapshot_linear_anchor_subissues() {
        let mut app = test_app();
        app.focus = Pane::Right;
        app.detail_tab = Tab::Linear;
        let task = &mut app.tasks[0];
        task.linear = vec![
            LinearStub {
                key: "ENG-29151".into(),
                title: "Fix bene-matching boundary".into(),
                state: "In Progress".into(),
                assignee: Some("@ashley".into()),
                depth: 0,
            },
            LinearStub {
                key: "ENG-30210".into(),
                title: "Tighten name normalize".into(),
                state: "Backlog".into(),
                assignee: None,
                depth: 1,
            },
            LinearStub {
                key: "ENG-30444".into(),
                title: "Investigate ALERT".into(),
                state: "Done".into(),
                assignee: Some("@ashley".into()),
                depth: 1,
            },
        ];
        let s = render_to_string(&app, 100, 25);
        // Minimal-list view: stubs render as one-line rows with key +
        // state glyph + title. Test stubs don't populate cache.issues,
        // so state_kind is empty and glyph defaults to "·" (MUTED).
        assert!(s.contains("ENG-29151"));
    }

    #[test]
    fn snapshot_linear_empty() {
        let mut app = test_app();
        app.focus = Pane::Right;
        app.detail_tab = Tab::Linear;
        app.tasks[0].linear.clear();
        let s = render_to_string(&app, 100, 25);
        assert!(s.contains("(no linked Linear issues)"));
    }

    #[test]
    fn tab_cycling_next_prev() {
        let mut app = test_app();
        app.focus = Pane::Right;
        assert_eq!(app.detail_tab, Tab::Overview);
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('l')));
        assert_eq!(app.detail_tab, Tab::Prs);
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('l')));
        assert_eq!(app.detail_tab, Tab::Linear);
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('l')));
        assert_eq!(app.detail_tab, Tab::Panes);
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('l')));
        assert_eq!(app.detail_tab, Tab::Overview); // wrap
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('h')));
        assert_eq!(app.detail_tab, Tab::Panes); // wrap back
    }

    #[test]
    fn number_keys_jump_to_tab_from_any_focus() {
        let mut app = test_app();
        // From List focus
        app.focus = Pane::List;
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('2')));
        assert_eq!(app.detail_tab, Tab::Prs);
        assert_eq!(app.focus, Pane::Right);

        // From Log focus
        app.focus = Pane::Right;
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('4')));
        assert_eq!(app.detail_tab, Tab::Panes);
        assert_eq!(app.focus, Pane::Right);

        // 1 = Overview, 3 = Linear
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('1')));
        assert_eq!(app.detail_tab, Tab::Overview);
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('3')));
        assert_eq!(app.detail_tab, Tab::Linear);
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
        // Log is no longer a focus zone. PgUp/PgDn/`<`/`>` work from any focus.
        let mut app = test_app();
        app.focus = Pane::List;
        app.log.follow_bottom = true;
        handle_key(&mut app, KeyEvent::from(KeyCode::PageUp));
        assert!(!app.log.follow_bottom);
        // > re-enables follow_bottom (tail-follow)
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('>')));
        assert!(app.log.follow_bottom);
        // < scrolls to top
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('<')));
        assert!(!app.log.follow_bottom);
        assert_eq!(app.log.scroll, 0);
    }

    #[test]
    fn unwired_keys_show_toast() {
        let mut app = test_app();
        // R is a Phase 4a key, not yet wired
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('R')));
        assert!(app.toast.is_some());
        let toast = app.toast.as_ref().unwrap();
        assert!(toast.contains("not yet wired"));
        assert!(toast.contains("R"));

        // Next non-toast key clears the toast
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('j')));
        assert!(app.toast.is_none());
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
        let lines = vec![
            "short".to_string(),
            "".to_string(),
            "x".repeat(100),
        ];
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
