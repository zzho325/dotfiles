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
    collections::{HashMap, HashSet},
    io::{self, stdout},
    process::{Command, Stdio},
    sync::OnceLock,
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

use crate::cache;
use crate::state::{self, TaskStatus, load_tmux_sessions};
use crate::store::{self, DesiredState, TaskRecord};

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
/// Stronger highlight for the focused-pane cursor row. Visible enough
/// to read at a glance against Rosé Pine Dawn's `0xfaf4ed` base.
const HL_MED: Color = Color::Rgb(0xe5, 0xd5, 0xc4);
/// Pale green-tinted background for `+` lines in the diff body.
const DIFF_ADD_BG: Color = Color::Rgb(0xea, 0xf0, 0xe2);
/// Pale rose-tinted background for `-` lines.
const DIFF_DEL_BG: Color = Color::Rgb(0xf6, 0xe2, 0xe2);

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
    pub record: TaskRecord,
    pub status: TaskStatus,
    pub prs: Vec<state::PrData>,
    pub panes: Vec<TmuxPaneInfo>,
    pub linear: Vec<LinearStub>,
}

impl TaskView {
    pub fn id(&self) -> store::TaskId {
        self.record.id
    }

    pub fn drift(&self) -> bool {
        self.record.drift.any()
    }
}

#[derive(Debug, Clone)]
pub struct LinearStub {
    pub key: String,
    pub title: String,
    pub state: String,
    pub assignee: Option<String>,
    pub depth: u8,
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

/// Linear tab sub-state. The flat list is the default; pressing Enter
/// pushes a Detail view (one issue, full page). Esc pops back. Stack
/// supports drill-into-sub-issue → drill-into-sub-issue chains.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinearView {
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

/// PR tab sub-state. Mirrors `LinearView` shape — minus the drill stack
/// since PRs don't nest.
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
    /// Linear tab sub-state.
    pub linear_view: LinearView,
    /// Row offset at the top of the Linear list viewport. Mutated by
    /// `render_linear_list` each frame using a sticky-margin scroll
    /// (same algorithm as `monitor`'s tui). Survives re-render even
    /// when rows reshape.
    pub linear_list_offset: usize,
    /// PR tab sub-state.
    pub pr_view: PrView,
    /// Per-PR persisted detail-view position. Esc-out saves into this
    /// map, drilling back into the same PR restores. Only `(file_cursor,
    /// scroll)` survive; `focus` resets to `Files`.
    pub pr_detail_state: HashMap<u32, (usize, u16)>,
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
            linear_list_offset: 0,
            pr_view: PrView::default(),
            pr_detail_state: HashMap::new(),
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
                let linear = linear_from_record(&record, &linear_cache);

                TaskView {
                    name,
                    record,
                    status,
                    prs,
                    panes,
                    linear,
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

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    // Fullscreen take-over: when drilled into a PR, the diff gets the
    // whole terminal. Esc returns to the three-pane layout. The detail
    // tab must still be Prs — if the user pressed `1`/`3`/`4` to jump
    // to another tab while drilled, we treat that as an exit (the
    // dispatcher resets pr_view) so this short-circuit is consistent.
    if let PrView::Detail { number, focus, file_cursor, scroll } = &app.pr_view {
        if app.detail_tab == Tab::Prs {
            render_pr_detail_fullscreen(
                frame, area, app, *number, *focus, *file_cursor, *scroll,
            );
            if app.show_help {
                render_help_overlay_pr_detail(frame, area);
            }
            if app.message_input.is_some() {
                render_message_input(frame, area, app);
            }
            return;
        }
    }

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
    if app.message_input.is_some() {
        render_message_input(frame, area, app);
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
            Style::default().fg(MUTED),
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
        Span::styled(prompt, Style::default().fg(LOVE)),
        Span::styled(buf.clone(), Style::default().fg(TEXT)),
        Span::styled("▌", Style::default().fg(LOVE)),
    ]);
    frame.render_widget(
        Paragraph::new(line).wrap(Wrap { trim: false }),
        input_area,
    );
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

            // P/L counts intentionally omitted from the list — the
            // user only cared about drift state at a glance, and the
            // detail tabs already surface PR/Linear counts.
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
                    Style::default().fg(if selected { LOVE } else { MUTED }),
                ),
                Span::styled(id_text, Style::default().fg(MUTED)),
                Span::styled(name_str, Style::default().fg(name_color)),
                Span::raw(" ".repeat(pad)),
            ];
            if !counts.is_empty() {
                let counts_color = if task.drift() && counts.starts_with(" ⚠") {
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

fn render_details(frame: &mut Frame, area: Rect, app: &mut App) {
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
        Some(t) => t.clone(),
        None => return,
    };

    match app.detail_tab {
        Tab::Overview => render_tab_overview(frame, body_area, app, &task),
        Tab::Prs => render_tab_prs(frame, body_area, app, &task),
        Tab::Linear => render_tab_linear(frame, body_area, app, &task),
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
        task.record.worktree.path.clone()
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
    if task.record.attention.needs_input {
        lines.push(kv_line(" attention:", "needs input"));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
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
            Style::default().fg(SUBTLE),
        ));
        lines.push(Line::styled(
            " orch pr add <task> <number>",
            Style::default().fg(MUTED),
        ));
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
        return;
    }

    let width = area.width as usize;
    for pr in &task.prs {
        let selected = focused && pr.number == cursor_number;
        let cursor_glyph = if selected { " ▸ " } else { "   " };
        let cursor_color = if selected { LOVE } else { MUTED };
        let id_color = if selected { LOVE } else { IRIS };

        let title = if pr.title.is_empty() {
            "(no title cached)".into()
        } else {
            pr.title.clone()
        };

        // State badge right-aligned so a wrapped branch name doesn't merge with it.
        let state_badge: Option<(&str, ratatui::style::Color)> = match pr.state.as_str() {
            "MERGED" => Some(("merged", IRIS)),
            "CLOSED" => Some(("closed", MUTED)),
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
            Span::styled(title_str, Style::default().fg(TEXT)),
            Span::raw(" ".repeat(pad)),
        ];
        if let Some((t, color)) = state_badge {
            row1.push(Span::styled(t.to_string(), Style::default().fg(color)));
        }
        lines.push(Line::from(row1));

        // Row 2: meta strip — ci · review · codex · age · branch (truncated).
        let mut meta: Vec<Span> = vec![Span::raw("    ")];
        meta.push(match pr.ci_pass {
            Some(true) => Span::styled("✓ ci", Style::default().fg(PINE)),
            Some(false) => Span::styled("✗ ci", Style::default().fg(LOVE)),
            None => Span::styled("· ci", Style::default().fg(MUTED)),
        });
        meta.push(if pr.approved {
            Span::styled("  ·  ✓ review", Style::default().fg(PINE))
        } else {
            Span::styled("  ·  · review", Style::default().fg(MUTED))
        });
        meta.push(match pr.codex {
            crate::state::CodexStatus::ThumbsUp => {
                Span::styled("  ·  ✓ codex", Style::default().fg(PINE))
            }
            crate::state::CodexStatus::Commented => {
                Span::styled("  ·  · codex commented", Style::default().fg(GOLD))
            }
            crate::state::CodexStatus::None => {
                Span::styled("  ·  · codex", Style::default().fg(MUTED))
            }
        });
        let age = relative_age(&pr.updated_at);
        if !age.is_empty() {
            meta.push(Span::styled(
                format!("  ·  {age}"),
                Style::default().fg(MUTED),
            ));
        }
        if !pr.head_branch.is_empty() {
            // Branch can be long ("ashley/ENG-29187-scrub-…"). Truncate
            // so the row fits on one terminal line.
            let branch_room = 36;
            let branch = truncate(&pr.head_branch, branch_room);
            meta.push(Span::styled(
                format!("  ·  {branch}"),
                Style::default().fg(MUTED),
            ));
        }
        // Mergeable: glyph only on conflict (skip cell noise on green path).
        if pr.mergeable.as_deref() == Some("CONFLICTING") {
            meta.push(Span::styled("  ⚠ conflict", Style::default().fg(GOLD)));
        }
        lines.push(Line::from(meta));

        // Row 3: stats — only when we have churn data.
        if pr.changed_files > 0 || pr.additions > 0 || pr.deletions > 0 {
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(
                    format!("+{} / -{}", pr.additions, pr.deletions),
                    Style::default().fg(SUBTLE),
                ),
                Span::styled(
                    format!("  ·  {} files", pr.changed_files),
                    Style::default().fg(MUTED),
                ),
            ]));
        }
        lines.push(Line::raw(""));
    }
    if focused {
        lines.push(Line::styled(
            " j/k move · Enter open · o browser",
            Style::default().fg(MUTED),
        ));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn render_tab_linear(frame: &mut Frame, area: Rect, app: &mut App, task: &TaskView) {
    let cache = crate::cache::read_linear();
    let focused = app.focus == Pane::Right && app.detail_tab == Tab::Linear;
    match &app.linear_view {
        LinearView::Detail { stack, sub_cursor } if !stack.is_empty() => {
            let stack = stack.clone();
            let sub_cursor = *sub_cursor;
            render_linear_detail(frame, area, &stack, sub_cursor, &cache, focused);
        }
        LinearView::List { cursor_key, pinned } => {
            let cursor_key = cursor_key.clone();
            let pinned = pinned.clone();
            render_linear_list(
                frame,
                area,
                focused,
                &mut app.linear_list_offset,
                task,
                &cursor_key,
                &pinned,
                &cache,
            );
        }
        _ => {
            let empty = HashSet::new();
            render_linear_list(
                frame,
                area,
                focused,
                &mut app.linear_list_offset,
                task,
                "",
                &empty,
                &cache,
            );
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

/// User identifier (Linear displayName) for the assignee filter.
/// First checks `ORCH_LINEAR_USER`; falls back to the most common
/// assignee in the cache (in a personal tool, that's the user).
/// Empty string disables filtering.
fn linear_me(cache: &crate::cache::LinearCache) -> String {
    if let Ok(s) = std::env::var("ORCH_LINEAR_USER") {
        if !s.is_empty() {
            return s;
        }
    }
    // Auto-detect: count assignees across cache.issues, pick the
    // mode. With one user this is the user; the cache is small so
    // counting per-render is fine.
    let mut counts: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for c in cache.issues.values() {
        if !c.assignee.is_empty() {
            *counts.entry(c.assignee.clone()).or_insert(0) += 1;
        }
    }
    counts
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .map(|(name, _)| name)
        .unwrap_or_default()
}

/// True iff this issue is "mine" — unassigned or assigned to me.
/// When `me` is empty, every issue passes (filter disabled).
fn is_mine(assignee: &str, me: &str) -> bool {
    me.is_empty() || assignee.is_empty() || assignee == me
}

/// Build the flat row stream. Sub-issues are ALWAYS expanded inline
/// — the list never reshapes when the cursor moves. Project headers
/// appear only when there are 2+ distinct projects. Top-level stubs
/// that are also a child of another linked stub are shown only as
/// the sub-issue (deduped) so the cursor doesn't have two rows
/// claiming the same key. Issues assigned to someone other than
/// `linear_me()` are dropped (both top-level and as sub-issues).
fn build_linear_rows(
    stubs: &[LinearStub],
    cache: &crate::cache::LinearCache,
    _cursor_key: &str,
    _pinned: &HashSet<String>,
) -> Vec<ListRow> {
    if stubs.is_empty() {
        return Vec::new();
    }

    let me = linear_me(cache);

    // Collect all keys that appear as a child of another linked stub.
    // These are skipped at top-level so the cursor sees one row per key.
    let child_keys: HashSet<String> = stubs
        .iter()
        .filter_map(|s| cache.issues.get(&s.key))
        .flat_map(|c| c.children.iter().map(|ch| ch.identifier.clone()))
        .collect();

    // Filter "not mine" — issues assigned to someone other than me
    // (unassigned issues always pass; treated as mine-by-default).
    let assignee_of = |key: &str| -> String {
        cache
            .issues
            .get(key)
            .map(|c| c.assignee.clone())
            .unwrap_or_default()
    };

    // A stub is shown iff: it's not also someone else's child (dedupe),
    // it's mine (or unassigned), and Linear actually knows about it
    // (not in `not_found`). Stale/orphan link entries are silently
    // hidden — `orch linear rm <task> <key>` cleans them up.
    let visible = |key: &str| -> bool {
        if child_keys.contains(key) {
            return false;
        }
        if cache.not_found.contains(&key.to_string()) {
            return false;
        }
        is_mine(&assignee_of(key), &me)
    };

    let mut projects: Vec<String> = Vec::new();
    for s in stubs {
        if !visible(&s.key) {
            continue;
        }
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

    let mut rows: Vec<ListRow> = Vec::new();
    let mut prev_project: Option<String> = None;

    for stub in stubs {
        if !visible(&stub.key) {
            continue;
        }
        let cached = cache.issues.get(&stub.key);
        let project_name = cached
            .and_then(|c| c.project.as_ref())
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "(no project)".into());

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

        rows.push(ListRow {
            key: stub.key.clone(),
            kind: RowKind::Parent { collapsed_subs: 0 },
            title: cached.map(|c| c.title.clone()).unwrap_or_else(|| stub.title.clone()),
            state_kind: cached.map(|c| c.state_kind.clone()).unwrap_or_default(),
            state_name: cached.map(|c| c.state.clone()).unwrap_or_default(),
            project_for_strip: project_name.clone(),
            not_found: is_not_found,
        });

        if n_children > 0 {
            // Sub-issues filtered by the same is_mine rule as top-level.
            let visible_children: Vec<_> = cached
                .unwrap()
                .children
                .iter()
                .filter(|ch| is_mine(&ch.assignee, &me))
                .collect();
            let n_visible = visible_children.len();
            for (i, child) in visible_children.iter().enumerate() {
                rows.push(ListRow {
                    key: child.identifier.clone(),
                    kind: RowKind::SubIssue {
                        is_last: i + 1 == n_visible,
                    },
                    title: child.title.clone(),
                    state_kind: child.state_kind.clone(),
                    state_name: child.state.clone(),
                    project_for_strip: project_name.clone(),
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
    focused: bool,
    list_offset: &mut usize,
    task: &TaskView,
    cursor_key: &str,
    pinned: &HashSet<String>,
    cache: &crate::cache::LinearCache,
) {
    if task.linear.is_empty() {
        let lines = vec![
            Line::raw(""),
            Line::styled(
                " (no linked Linear issues)",
                Style::default().fg(SUBTLE),
            ),
            Line::styled(
                " orch linear add <task> ENG-123  ·  orch linear scan <task>",
                Style::default().fg(MUTED),
            ),
        ];
        frame.render_widget(Paragraph::new(lines), area);
        return;
    }

    let all_rows = build_linear_rows(&task.linear, cache, cursor_key, pinned);
    let width = area.width as usize;

    // Sticky-margin scroll — same shape as monitor::tui_render::render_list.
    // Cursor stays at least `margin` rows from each viewport edge, except
    // at the absolute top/bottom of the list.
    let footer_reserved = if focused { 2 } else { 0 };
    let viewport_height = (area.height as usize).saturating_sub(footer_reserved);
    let cursor_row = all_rows
        .iter()
        .position(|r| r.key == cursor_key)
        .unwrap_or(0);
    let margin = 3usize.min(viewport_height / 3);
    let mut offset = *list_offset;
    if cursor_row < offset + margin {
        offset = cursor_row.saturating_sub(margin);
    } else if viewport_height > 0 && cursor_row + margin + 1 > offset + viewport_height {
        offset = cursor_row + margin + 1 - viewport_height;
    }
    offset = offset.min(all_rows.len().saturating_sub(viewport_height));
    *list_offset = offset;
    let start = offset;
    let end = (start + viewport_height).min(all_rows.len());
    let rows: &[ListRow] = &all_rows[start..end];

    let mut lines: Vec<Line> = Vec::new();

    let mut last_was_header = false;
    for row in rows {
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
            RowKind::Parent { collapsed_subs: _ } => {
                let selected = focused && row.key == cursor_key;
                let cursor_glyph = if selected { " ▸ " } else { "   " };
                let cursor_color = if selected { LOVE } else { MUTED };
                let state_color = linear_state_color(&row.state_name);
                let glyph = state_glyph(&row.state_kind);
                let title = strip_project_prefix(&row.title, &row.project_for_strip);
                lines.push(compose_row(
                    cursor_glyph,
                    cursor_color,
                    &row.key,
                    glyph,
                    state_color,
                    &title,
                    if row.not_found { Some(LOVE) } else { None },
                    None,
                    width,
                    selected,
                ));
                last_was_header = false;
            }
            RowKind::SubIssue { is_last } => {
                let selected = focused && row.key == cursor_key;
                // Sub-issues indented 4 cells deeper than parents so
                // the hierarchy reads at a glance (parent key column 3,
                // sub-issue key column 7).
                let last = *is_last;
                let prefix = if selected {
                    "     ▸ "
                } else if last {
                    "     └ "
                } else {
                    "     │ "
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
            " j/k move · Enter open · o browser",
            Style::default().fg(MUTED),
        ));
    }

    // No wrap on the list — each row is intended to be one line; long
    // titles are truncated in compose_row. Wrapping shifted alignment
    // when titles got cut.
    frame.render_widget(Paragraph::new(lines), area);
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
    focused: bool,
) {
    let key = stack.last().cloned().unwrap_or_default();
    let cached = cache.issues.get(&key);

    let mut lines: Vec<Line> = Vec::new();

    let Some(c) = cached else {
        // Sub-issues live as `children` on their parent's cache entry,
        // not as top-level `issues` keys. Render the limited data we
        // have rather than a perpetual "loading…".
        if let Some((parent_key, child)) = find_child_in_cache(cache, &key) {
            render_child_detail(frame, area, &key, parent_key, child, focused);
            return;
        }
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

/// When the user has the Linear tab focused with a cursored row, the
/// log pane renders that issue's preview instead of orchestrator
/// runs. Returns the key + a discriminator (List vs Detail-stack-top)
/// when a preview should render; None to fall back to log.
fn linear_preview_target(app: &App) -> Option<(String, &'static str)> {
    if app.focus != Pane::Right || app.detail_tab != Tab::Linear {
        return None;
    }
    match &app.linear_view {
        LinearView::List { cursor_key, .. } if !cursor_key.is_empty() => {
            Some((cursor_key.clone(), "list"))
        }
        _ => None,
    }
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
                Style::default().fg(GOLD),
            )),
            Rect { x: area.x, y: toast_y, width: area.width, height: 1 },
        );
    }
}

/// PR detail view — Linear-flavored two-pane: file list left, diff right.
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
    let state_badge: Option<(&str, ratatui::style::Color)> = pr.and_then(|p| {
        match p.state.as_str() {
            "MERGED" => Some(("merged", IRIS)),
            "CLOSED" => Some(("closed", MUTED)),
            _ => None,
        }
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
        Span::styled(id_text, Style::default().fg(IRIS)),
        Span::raw("  "),
        Span::styled(title_str, Style::default().fg(TEXT)),
        Span::raw(" ".repeat(title_pad)),
    ];
    if let Some((t, color)) = state_badge {
        row0.push(Span::styled(t.to_string(), Style::default().fg(color)));
    }
    frame.render_widget(
        Paragraph::new(Line::from(row0)),
        Rect { x: area.x, y: area.y, width: area.width, height: 1 },
    );

    // Row 1 — meta strip. Single cadence: `  ·  ` between groups.
    let mut meta: Vec<Span> = vec![Span::raw(" ")];
    if let Some(c) = pr {
        meta.push(match c.ci_pass {
            Some(true) => Span::styled("✓ ci", Style::default().fg(PINE)),
            Some(false) => Span::styled("✗ ci", Style::default().fg(LOVE)),
            None => Span::styled("· ci", Style::default().fg(MUTED)),
        });
        meta.push(if c.approved {
            Span::styled("  ·  ✓ review", Style::default().fg(PINE))
        } else {
            Span::styled("  ·  · review", Style::default().fg(MUTED))
        });
        meta.push(match c.codex.as_str() {
            "ThumbsUp" => Span::styled("  ·  ✓ codex", Style::default().fg(PINE)),
            "Commented" => Span::styled("  ·  · codex commented", Style::default().fg(GOLD)),
            _ => Span::styled("  ·  · codex", Style::default().fg(MUTED)),
        });
        let age = relative_age(&c.updated_at);
        if !age.is_empty() {
            meta.push(Span::styled(
                format!("  ·  {age}"),
                Style::default().fg(MUTED),
            ));
        }
        if !c.head_branch.is_empty() {
            meta.push(Span::styled(
                format!("  ·  {} → main", truncate(&c.head_branch, 36)),
                Style::default().fg(MUTED),
            ));
        }
        meta.push(Span::styled(
            format!("  ·  +{} / -{}", c.additions, c.deletions),
            Style::default().fg(SUBTLE),
        ));
        meta.push(Span::styled(
            format!("  ·  {} files", c.changed_files),
            Style::default().fg(MUTED),
        ));
        if c.mergeable.as_deref() == Some("CONFLICTING") {
            meta.push(Span::styled("  ·  ⚠ conflict", Style::default().fg(GOLD)));
        }
    }
    frame.render_widget(
        Paragraph::new(Line::from(meta)),
        Rect { x: area.x, y: area.y + 1, width: area.width, height: 1 },
    );

    // Row 2 — blank gap.
    let mut body_top: u16 = 3;

    // Optional row 3 — stale-diff banner.
    let stale = match (pr, diff) {
        (Some(p), Some(d)) if !p.head_sha.is_empty()
            && !d.head_sha.is_empty()
            && p.head_sha != d.head_sha => true,
        _ => false,
    };
    if stale {
        frame.render_widget(
            Paragraph::new(Line::styled(
                " diff stale (head moved) · r refresh",
                Style::default().fg(GOLD),
            )),
            Rect { x: area.x, y: area.y + body_top, width: area.width, height: 1 },
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
            PrDetailFocus::Files => " j/k file · Tab diff · ]/[ next/prev · r refresh · o browser · Esc back",
            PrDetailFocus::Diff => " j/k scroll · H/L hunk · Tab files · ]/[ next/prev · r refresh · o browser · Esc back",
        };
        frame.render_widget(
            Paragraph::new(Line::styled(hint, Style::default().fg(MUTED))),
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
            lines.push(Line::styled(" PR metadata not yet fetched.", Style::default().fg(MUTED)));
            lines.push(Line::styled(
                " Wait for the next PR loop cycle (~30s) or restart `orch daemon`.",
                Style::default().fg(SUBTLE),
            ));
        } else {
            lines.push(Line::styled(" diff loading…", Style::default().fg(MUTED)));
            lines.push(Line::styled(
                " (refreshing in the background; press r to retry)",
                Style::default().fg(SUBTLE),
            ));
        }
        frame.render_widget(Paragraph::new(lines), body_area);
        return;
    };

    if let Some(err) = &d.error {
        let mut lines = vec![Line::raw("")];
        lines.push(Line::styled(
            format!(" diff fetch failed: {err}"),
            Style::default().fg(LOVE),
        ));
        lines.push(Line::styled(
            " r retry · o browser · Esc back",
            Style::default().fg(MUTED),
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
            Style::default().fg(GOLD),
        ));
        lines.push(Line::styled(
            " press o to open in browser",
            Style::default().fg(MUTED),
        ));
        frame.render_widget(Paragraph::new(lines), body_area);
        return;
    }

    if d.files.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::styled(" no changes", Style::default().fg(MUTED))),
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
        lines.push(Line::styled(label, Style::default().fg(MUTED)));
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
        let glyph_color = if cur_focused { LOVE } else { MUTED };
        let path_color = if is_cur { TEXT } else { SUBTLE };
        let stats = format!("+{}/-{}", f.additions, f.deletions);
        // path + stats fits in `area.width` minus glyph (2) + " " + stats.
        let stats_room = stats.chars().count();
        let path_room = (area.width as usize).saturating_sub(2 + 1 + stats_room);
        let path = truncate_tail(stripped[i], path_room);
        let pad = path_room.saturating_sub(path.chars().count());

        let base_style = if is_cur && focused {
            Style::default().bg(HL_LOW)
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
            Span::styled(stats, base_style.fg(MUTED)),
        ];
        // Pad the highlight bar across the full column width so it
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
        Span::styled(file.path.clone(), Style::default().fg(TEXT)),
    ]));
    if file.status != "modified" {
        lines.push(Line::styled(
            format!(" ({})", file.status),
            Style::default().fg(MUTED),
        ));
    }
    lines.push(Line::raw(""));

    if file.status == "binary" {
        lines.push(Line::styled(
            " binary file · diff suppressed",
            Style::default().fg(MUTED),
        ));
        return (lines, hunk_anchors);
    }

    for hunk in &file.hunks {
        hunk_anchors.push(lines.len() as u16);
        if let Some((header_part, ctx)) = split_hunk_header(&hunk.header) {
            let mut spans = vec![
                Span::raw(" "),
                Span::styled(header_part.to_string(), Style::default().fg(MUTED)),
            ];
            if !ctx.is_empty() {
                spans.push(Span::styled(
                    format!(" {ctx}"),
                    Style::default().fg(SUBTLE),
                ));
            }
            lines.push(Line::from(spans));
        } else {
            lines.push(Line::styled(
                format!(" {}", hunk.header),
                Style::default().fg(MUTED),
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
            // Glyph in PINE/LOVE; body stays in TEXT/SUBTLE.
            let (prefix, rest, glyph_color, body_color, bg) =
                if let Some(rest) = line.strip_prefix('+') {
                    ("+", rest, PINE, TEXT, Some(DIFF_ADD_BG))
                } else if let Some(rest) = line.strip_prefix('-') {
                    ("-", rest, LOVE, TEXT, Some(DIFF_DEL_BG))
                } else if let Some(rest) = line.strip_prefix(' ') {
                    (" ", rest, MUTED, SUBTLE, None)
                } else {
                    (" ", line.as_str(), MUTED, MUTED, None)
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
/// a long path overflows the column.
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
/// stats, top files by churn. Linear-flavored — content-forward, no
/// dividers.
fn render_pr_preview(frame: &mut Frame, area: Rect, number: u32) {
    let pr_cache = crate::cache::read_prs();
    let diff_cache = crate::cache::read_pr_diffs();
    let cached = pr_cache.prs.get(&number);
    let diff = diff_cache.diffs.get(&number);

    let header = format!(" preview: #{number}");
    frame.render_widget(
        Paragraph::new(Line::styled(header, Style::default().fg(MUTED))),
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
        lines.push(Line::styled(" loading…", Style::default().fg(MUTED)));
        frame.render_widget(Paragraph::new(lines), body_area);
        return;
    };

    // Title.
    if !c.title.is_empty() {
        lines.push(Line::from(vec![
            Span::raw(" "),
            Span::styled(c.title.clone(), Style::default().fg(TEXT)),
        ]));
    }

    // Meta strip.
    let mut meta: Vec<Span> = vec![Span::raw(" ")];
    meta.push(Span::styled(
        format!("#{}", c.number),
        Style::default().fg(IRIS),
    ));
    meta.push(Span::raw("  "));
    meta.push(match c.ci_pass {
        Some(true) => Span::styled("✓ ci", Style::default().fg(PINE)),
        Some(false) => Span::styled("✗ ci", Style::default().fg(LOVE)),
        None => Span::styled("· ci", Style::default().fg(MUTED)),
    });
    meta.push(if c.approved {
        Span::styled("  ·  ✓ review", Style::default().fg(PINE))
    } else {
        Span::styled("  ·  · review", Style::default().fg(MUTED))
    });
    meta.push(match c.codex.as_str() {
        "ThumbsUp" => Span::styled("  ·  ✓ codex", Style::default().fg(PINE)),
        "Commented" => Span::styled("  ·  · codex commented", Style::default().fg(GOLD)),
        _ => Span::styled("  ·  · codex", Style::default().fg(MUTED)),
    });
    let age = relative_age(&c.updated_at);
    if !age.is_empty() {
        meta.push(Span::styled(
            format!("  ·  {age}"),
            Style::default().fg(MUTED),
        ));
    }
    if !c.head_branch.is_empty() {
        meta.push(Span::styled(
            format!("  ·  {}", truncate(&c.head_branch, 36)),
            Style::default().fg(MUTED),
        ));
    }
    lines.push(Line::from(meta));

    // Stats row.
    let mut stats: Vec<Span> = vec![Span::raw(" ")];
    stats.push(Span::styled(
        format!("+{} / -{}", c.additions, c.deletions),
        Style::default().fg(SUBTLE),
    ));
    stats.push(Span::styled(
        format!("  ·  {} files", c.changed_files),
        Style::default().fg(MUTED),
    ));
    let merge_glyph = match c.mergeable.as_deref() {
        Some("CONFLICTING") => Some(("  ·  ⚠ conflict", GOLD)),
        Some("MERGEABLE") => None,
        _ => None,
    };
    if let Some((s, color)) = merge_glyph {
        stats.push(Span::styled(s, Style::default().fg(color)));
    }
    match c.state.as_str() {
        "MERGED" => stats.push(Span::styled("  ·  merged", Style::default().fg(IRIS))),
        "CLOSED" => stats.push(Span::styled("  ·  closed", Style::default().fg(MUTED))),
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
                Span::styled(w.clone(), Style::default().fg(SUBTLE)),
            ]));
        }
        if wrapped.len() > take && take > 0 {
            lines.push(Line::styled(" …", Style::default().fg(MUTED)));
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
                        Style::default().fg(SUBTLE),
                    ),
                    Span::styled(
                        format!("  +{}/-{}", f.additions, f.deletions),
                        Style::default().fg(MUTED),
                    ),
                ]));
            }
            let extra = by_churn.len().saturating_sub(take);
            if extra > 0 {
                lines.push(Line::styled(
                    format!(" ({extra} more)"),
                    Style::default().fg(MUTED),
                ));
            }
        }
    }

    frame.render_widget(Paragraph::new(lines), body_area);
}

/// Render an issue preview into the log-pane area: title, meta line,
/// description (wrapped, truncated to fit). Source of truth is the
/// shared `linear.json` cache. Sub-issues aren't fetched as top-level
/// entries; their preview is rendered from the parent's `children`
/// data (title, state, assignee — the subset Linear returns inline).
fn render_linear_preview(frame: &mut Frame, area: Rect, key: &str, _kind: &str) {
    let cache = crate::cache::read_linear();
    let cached = cache.issues.get(key);

    let header = format!(" preview: {key}");
    frame.render_widget(
        Paragraph::new(Line::styled(header, Style::default().fg(MUTED))),
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
        if let Some((parent_key, child)) = find_child_in_cache(&cache, key) {
            render_child_preview(frame, body_area, parent_key, child);
            return;
        }
        if cache.not_found.contains(&key.to_string()) {
            lines.push(Line::styled(
                " not on Linear",
                Style::default().fg(LOVE),
            ));
        } else {
            lines.push(Line::styled(
                " loading…",
                Style::default().fg(MUTED),
            ));
        }
        frame.render_widget(Paragraph::new(lines), body_area);
        return;
    };

    // Title
    lines.push(Line::from(vec![
        Span::raw(" "),
        Span::styled(c.title.clone(), Style::default().fg(TEXT)),
    ]));

    // Meta: state · age · assignee · project · sub-count
    let state_color = linear_state_color(&c.state);
    let mut meta: Vec<Span> = vec![
        Span::raw(" "),
        Span::styled(
            format!("{} {}", state_glyph(&c.state_kind), c.state),
            Style::default().fg(state_color),
        ),
    ];
    let age = relative_age(&c.updated_at);
    if !age.is_empty() {
        meta.push(Span::styled(
            format!("  ·  {age}"),
            Style::default().fg(MUTED),
        ));
    }
    if !c.assignee.is_empty() {
        meta.push(Span::styled(
            format!("  ·  @{}", c.assignee),
            Style::default().fg(MUTED),
        ));
    }
    if let Some(p) = &c.project {
        meta.push(Span::styled(
            format!("  ·  {}", p.name),
            Style::default().fg(MUTED),
        ));
    }
    if let Some(parent_key) = &c.parent_key {
        let title = c.parent_title.clone().unwrap_or_default();
        meta.push(Span::styled(
            format!("  ·  parent {parent_key} {title}"),
            Style::default().fg(MUTED),
        ));
    }
    lines.push(Line::from(meta));

    if !c.description.is_empty() {
        lines.push(Line::raw(""));
        let width = (body_area.width.saturating_sub(2) as usize).max(20);
        let wrapped = wrap_text(&c.description, width);
        let body_room = (body_area.height as usize).saturating_sub(lines.len());
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
        if wrapped.len() > take && take > 0 {
            lines.push(Line::styled(
                " …",
                Style::default().fg(MUTED),
            ));
        }
    }

    frame.render_widget(Paragraph::new(lines), body_area);
}

/// Find a sub-issue in the cache by walking parents' `children` arrays.
/// Returns `(parent_key, child)` on match. Sub-issues aren't fetched as
/// top-level cache entries — the only data we have is the inline subset
/// Linear returned with the parent.
fn find_child_in_cache<'a>(
    cache: &'a crate::cache::LinearCache,
    key: &str,
) -> Option<(&'a str, &'a crate::cache::CachedChild)> {
    for (parent_key, parent) in &cache.issues {
        if let Some(child) = parent
            .children
            .iter()
            .find(|c| c.identifier == key)
        {
            return Some((parent_key.as_str(), child));
        }
    }
    None
}

/// Render the Linear detail view for a sub-issue. Sub-issues only
/// carry the inline subset Linear returns with the parent (no
/// description, project, age) — show what we have plus a parent
/// pointer instead of stalling on "loading…".
fn render_child_detail(
    frame: &mut Frame,
    area: Rect,
    key: &str,
    parent_key: &str,
    child: &crate::cache::CachedChild,
    _focused: bool,
) {
    let mut lines: Vec<Line> = Vec::new();
    let state_color = linear_state_color(&child.state);
    lines.push(Line::from(vec![
        Span::styled(format!(" {key}"), Style::default().fg(IRIS)),
        Span::styled(
            format!(
                "  ·  {} {}",
                state_glyph(&child.state_kind),
                child.state,
            ),
            Style::default().fg(state_color),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::raw(" "),
        Span::styled(child.title.clone(), Style::default().fg(TEXT)),
    ]));
    lines.push(Line::raw(""));
    if !child.assignee.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(" Assignee  ", Style::default().fg(MUTED)),
            Span::styled(
                format!("@{}", child.assignee),
                Style::default().fg(TEXT),
            ),
        ]));
    }
    lines.push(Line::from(vec![
        Span::styled(" Parent    ", Style::default().fg(MUTED)),
        Span::styled(parent_key.to_string(), Style::default().fg(TEXT)),
    ]));
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        " (sub-issue inline data — full description requires open in browser)",
        Style::default().fg(MUTED),
    ));
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        " Esc back · u parent · o browser",
        Style::default().fg(MUTED),
    ));
    frame.render_widget(Paragraph::new(lines), area);
}

/// Render the limited preview available for a sub-issue: title, state,
/// assignee, and a pointer to the parent.
fn render_child_preview(
    frame: &mut Frame,
    body_area: Rect,
    parent_key: &str,
    child: &crate::cache::CachedChild,
) {
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::raw(" "),
        Span::styled(child.title.clone(), Style::default().fg(TEXT)),
    ]));
    let state_color = linear_state_color(&child.state);
    let mut meta: Vec<Span> = vec![
        Span::raw(" "),
        Span::styled(
            format!("{} {}", state_glyph(&child.state_kind), child.state),
            Style::default().fg(state_color),
        ),
    ];
    if !child.assignee.is_empty() {
        meta.push(Span::styled(
            format!("  ·  @{}", child.assignee),
            Style::default().fg(MUTED),
        ));
    }
    meta.push(Span::styled(
        format!("  ·  child of {parent_key}"),
        Style::default().fg(MUTED),
    ));
    lines.push(Line::from(meta));
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

/// Two-char ASCII label for a Linear state-kind category. Pure ASCII
/// for guaranteed 1-cell-per-char rendering across all terminals.
/// Color carries the state-kind distinction.
fn state_glyph(kind: &str) -> &'static str {
    match kind {
        "started" => "ip",
        "completed" => "dn",
        "canceled" => "cx",
        "unstarted" => "td",
        "backlog" => "bk",
        "triage" => "tr",
        _ => "··",
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
    // When cursor is on a Linear-tab list row, the log pane becomes
    // a preview of that issue (title + meta + description) so the
    // user can read details without drilling. Restores to the run
    // log as soon as focus leaves Linear.
    if let Some((key, cursor_kind)) = linear_preview_target(app) {
        render_linear_preview(frame, area, &key, &cursor_kind);
        return;
    }
    if let Some(number) = pr_preview_target(app) {
        render_pr_preview(frame, area, number);
        return;
    }

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
    frame.render_widget(
        ratatui::widgets::Clear,
        overlay,
    );

    let lines: Vec<Line> = if pr_detail {
        vec![
            Line::styled(" key bindings — PR detail (fullscreen)", Style::default().fg(LOVE)),
            Line::styled("─".repeat(w as usize), Style::default().fg(MUTED)),
            kv_line("  Esc      ", "back to PR list (saves position)"),
            kv_line("  Tab      ", "toggle Files ↔ Diff focus"),
            kv_line("  j / k    ", "Files: prev/next file  ·  Diff: scroll"),
            kv_line("  ] / [    ", "next / prev file (from any focus)"),
            kv_line("  H / L    ", "prev / next hunk (Diff focus only)"),
            kv_line("  r        ", "refresh diff for this PR"),
            kv_line("  o        ", "open PR in browser"),
            kv_line("  1-9      ", "attach to task #N"),
            kv_line("  q        ", "quit orch"),
            Line::styled(" Position is saved per PR — re-Enter restores cursor + scroll", Style::default().fg(MUTED)),
        ]
    } else {
        vec![
            Line::styled(" key bindings", Style::default().fg(LOVE)),
            Line::styled("─".repeat(w as usize), Style::default().fg(MUTED)),
            Line::styled(" Global", Style::default().fg(IRIS)),
            kv_line("  q        ", "quit"),
            kv_line("  Tab      ", "toggle list ↔ right"),
            kv_line("  [ / ]    ", "previous / next task (any focus)"),
            kv_line("  1-9      ", "attach to task #N"),
            kv_line("  Esc      ", "right → list; list → quit"),
            kv_line("  PgUp/Dn  ", "log scroll  ·  < top  ·  > tail"),
            kv_line("  ?  r  m  ", "help · refresh · message"),
            Line::styled(" List", Style::default().fg(IRIS)),
            kv_line("  j k g G  ", "move · top / bottom"),
            kv_line("  J K      ", "move task down / up in open_order"),
            kv_line("  s p R x  ", "spawn · pause · resume · close"),
            kv_line("  Enter    ", "attach to active pane"),
            Line::styled(" Right zone", Style::default().fg(IRIS)),
            kv_line("  j k      ", "move cursor in active tab"),
            kv_line("  Enter    ", "open / attach in active tab"),
            Line::styled(" Enter on a PR row → fullscreen lazygit-style diff", Style::default().fg(MUTED)),
        ]
    };

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

const UNWIRED_KEYS: &[char] = &['n', 'M', 'W'];

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
        // Navigate tasks from any focus — useful for cycling through
        // tasks while staying in a detail tab (Linear, PRs, etc) without
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
                reset_linear_cursor_for_new_task(app);
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
                reset_linear_cursor_for_new_task(app);
            }
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

fn lifecycle_op(
    app: &mut App,
    label: &str,
    f: fn(&str, &str) -> Result<String, String>,
) {
    let Some(task) = app.selected_task() else { return };
    let name = task.name.clone();
    let session = task.record.tmux.session_name.clone();
    match f(&name, &session) {
        Ok(msg) => app.toast = Some(msg),
        Err(e) => app.toast = Some(format!("{label} failed: {e}")),
    }
    // Re-pull task state so the row badge reflects the change.
    app.refresh_status();
}

/// Spawn a worker for the task: create a tmux session in the
/// worktree and start `claude '/orch:worker <md>'`. Idempotent —
/// no-ops with a friendly message if the session already exists.
fn lifecycle_spawn(name: &str, _session: &str) -> Result<String, String> {
    let session = format!("task-{name}");
    if find_actual_session(&session).is_some() {
        return Err(format!("session {session} already exists"));
    }
    let store = store::Store::default();
    let record = store
        .load_record_by_slug(name)
        .ok_or_else(|| format!("no task '{name}'"))?;
    let repo = std::env::var("ORCH_REPO")
        .map_err(|_| "ORCH_REPO not set".to_string())?;
    let work_dir = if !record.worktree.path.is_empty() {
        state::expand_home(&record.worktree.path)
    } else {
        format!("{repo}/task-{name}")
    };
    let task_file = state::tasks_dir().join(format!("{name}.md"));
    if !task_file.exists() {
        return Err(format!("no task file: {}", task_file.display()));
    }
    let cmd_str = format!("claude '/orch:worker {}'", task_file.display());
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
    let now = cache::now_epoch();
    store.update_record_by_slug(name, |r| {
        r.tmux.session_name = session.clone();
        r.worktree.path = work_dir.clone();
        r.desired_state = DesiredState::Active;
        if r.started_at.is_none() {
            r.started_at = Some(now);
        }
        r.updated_at = now;
    });
    Ok(format!("spawned {session}"))
}

fn lifecycle_resume(name: &str, session: &str) -> Result<String, String> {
    let store = store::Store::default();
    let now = cache::now_epoch();
    store.update_record_by_slug(name, |r| {
        r.desired_state = DesiredState::Active;
        r.updated_at = now;
    });
    let _ = session;
    lifecycle_spawn(name, "")
}

fn lifecycle_pause(name: &str, session: &str) -> Result<String, String> {
    if !session.is_empty() {
        if let Some(actual) = find_actual_session(session) {
            let _ = Command::new("tmux")
                .args(["kill-session", "-t", &actual])
                .stderr(Stdio::null())
                .status();
        }
    }
    let store = store::Store::default();
    let now = cache::now_epoch();
    store.update_record_by_slug(name, |r| {
        r.desired_state = DesiredState::Paused;
        r.paused_at = Some(now);
        r.updated_at = now;
    });
    Ok(format!("paused {name}"))
}

fn lifecycle_close(name: &str, session: &str) -> Result<String, String> {
    let store = store::Store::default();
    let Some(record) = store.load_record_by_slug(name) else {
        return Err(format!("no task '{name}'"))
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

    // 4. Remove worktree. Surface failures via the toast — a silent
    //    close that leaves the worktree behind is the orphan-worktree
    //    pain point we set out to fix.
    let warning = if worktree_path.is_empty() {
        None
    } else {
        let wt = state::expand_home(&worktree_path);
        let path = std::path::Path::new(&wt);
        if path.exists() {
            state::remove_worktree(path).err().map(|e| {
                format!("worktree {wt} not removed: {}", e.replace('\n', " "))
            })
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
        // h/l no-op in PR Detail fullscreen; Esc to leave.
        (_, KeyCode::Char('h')) | (_, KeyCode::Left) => {
            if !matches!(app.pr_view, PrView::Detail { .. })
                || app.detail_tab != Tab::Prs
            {
                app.detail_tab = app.detail_tab.prev();
                if app.detail_tab == Tab::Prs {
                    ensure_pr_cursor(app);
                } else if app.detail_tab == Tab::Linear {
                    ensure_linear_cursor(app);
                }
            }
        }
        (_, KeyCode::Char('l')) | (_, KeyCode::Right) => {
            if !matches!(app.pr_view, PrView::Detail { .. })
                || app.detail_tab != Tab::Prs
            {
                app.detail_tab = app.detail_tab.next();
                if app.detail_tab == Tab::Prs {
                    ensure_pr_cursor(app);
                } else if app.detail_tab == Tab::Linear {
                    ensure_linear_cursor(app);
                }
            }
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

/// Reset cursor_key when the user moves to a different task, since
/// the previously-cursored Linear key likely doesn't exist on the
/// new task. Also collapses pinned-open parents.
fn reset_linear_cursor_for_new_task(app: &mut App) {
    // Drop any drilled detail view and clear the list cursor so the
    // Linear pane re-anchors to the newly selected task's links.
    app.linear_view = LinearView::default();
    // Same for PRs — re-anchor cursor to first PR of new task.
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
                // If the cursored issue is a sub-issue, seed the stack
                // with its ancestors so Esc walks: cursor → parent →
                // grandparent → ... → list. Cap at 8 to match the drill
                // limit elsewhere in this file.
                let mut stack: Vec<String> = Vec::new();
                let mut cur = k.clone();
                while let Some(p) = cache
                    .issues
                    .get(&cur)
                    .and_then(|c| c.parent_key.clone())
                {
                    stack.push(p.clone());
                    cur = p;
                    if stack.len() >= 7 {
                        break;
                    }
                }
                stack.reverse();
                stack.push(k);
                app.linear_view = LinearView::Detail {
                    stack,
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
    let key: Option<String> = match &app.linear_view {
        LinearView::Detail { stack, .. } => stack.last().cloned(),
        LinearView::List { cursor_key, .. } if !cursor_key.is_empty() => {
            Some(cursor_key.clone())
        }
        _ => None,
    };
    let Some(key) = key else { return };

    // Top-level cache hit — use the canonical URL Linear returned.
    if let Some(c) = cache.issues.get(&key) {
        if !c.url.is_empty() {
            open_url(&c.url);
            return;
        }
    }

    // Sub-issue fallback: derive `https://linear.app/<ws>/issue/<KEY>`
    // from the parent's URL by stripping the slug. Linear redirects
    // bare-identifier URLs to the canonical slug.
    if let Some((parent_key, _)) = find_child_in_cache(&cache, &key) {
        if let Some(parent) = cache.issues.get(parent_key) {
            if let Some(prefix) = parent.url.split("/issue/").next() {
                if !prefix.is_empty() {
                    open_url(&format!("{prefix}/issue/{key}"));
                    return;
                }
            }
        }
    }

    // Last-resort fallback — Linear accepts the bare identifier path
    // even without a workspace prefix, redirecting to the user's last-
    // visited workspace. Better than silently failing.
    open_url(&format!("https://linear.app/issue/{key}"));
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
    attach(session, None);
}

fn attach_pane(session: &str, pane_id: &str) {
    attach(session, Some(pane_id));
}

fn attach(session: &str, pane_id: Option<&str>) {
    let Some(actual) = find_actual_session(session) else { return };
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
        app.linear_list_offset = 0;
    } else if app.detail_tab == Tab::Linear {
        ensure_linear_cursor(&mut app);
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
                        path: "/Users/a/column/task-infra-triage".into(),
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
                linear: vec![LinearStub {
                    key: "ENG-29151".into(),
                    title: "(stub: ENG-29151)".into(),
                    state: "—".into(),
                    assignee: None,
                    depth: 0,
                }],
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
                linear: vec![],
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
                linear: vec![],
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
            linear_list_offset: 0,
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
    fn task_navigation_resets_drilled_linear_view() {
        let mut app = test_app();
        // Simulate the user drilled into a Linear issue from the
        // previously-selected task.
        app.linear_view = LinearView::Detail {
            stack: vec!["ENG-29215".into()],
            sub_cursor: 0,
        };
        // Move task selection forward.
        reset_linear_cursor_for_new_task(&mut app);
        // Drilled state must be discarded so the Linear pane re-anchors.
        assert!(matches!(app.linear_view, LinearView::Detail { .. }) == false);
        match &app.linear_view {
            LinearView::List { cursor_key, pinned } => {
                assert!(cursor_key.is_empty());
                assert!(pinned.is_empty());
            }
            _ => panic!("expected LinearView::List after reset"),
        }
    }

    #[test]
    fn task_navigation_resets_list_cursor_and_pinned() {
        let mut app = test_app();
        // Cursor at a sub-issue, parent pinned open.
        let mut pinned: HashSet<String> = HashSet::new();
        pinned.insert("ENG-28816".into());
        app.linear_view = LinearView::List {
            cursor_key: "ENG-29215".into(),
            pinned,
        };
        reset_linear_cursor_for_new_task(&mut app);
        match &app.linear_view {
            LinearView::List { cursor_key, pinned } => {
                assert!(cursor_key.is_empty());
                assert!(pinned.is_empty());
            }
            _ => panic!("expected LinearView::List after reset"),
        }
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
        assert!(s.contains("attach to task #N"));
        assert!(s.contains("Enter on a PR row"));
    }

    #[test]
    fn pr_detail_hunk_jump_uses_real_anchors() {
        // Build a synthetic diff cache with two hunks so H/L can move
        // scroll between known anchor rows.
        use crate::cache::{
            CachedPrDiff, CachedPrDiffFile, CachedPrDiffHunk, PrDiffCache,
        };
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
    fn linear_enter_seeds_ancestor_stack() {
        // ENG-29151 is a child of ENG-28816 in the live test cache.
        // Entering on a sub-issue should seed the stack with parents
        // so Esc walks back through the hierarchy: cursor → parent → list.
        let mut app = test_app();
        app.focus = Pane::Right;
        app.detail_tab = Tab::Linear;
        app.linear_view = LinearView::List {
            cursor_key: "ENG-29151".into(),
            pinned: HashSet::new(),
        };
        handle_key(&mut app, KeyEvent::from(KeyCode::Enter));
        match &app.linear_view {
            LinearView::Detail { stack, sub_cursor } => {
                // Last in stack is what's shown; everything before it is
                // walked by Esc.
                assert_eq!(stack.last(), Some(&"ENG-29151".to_string()));
                assert!(
                    stack.contains(&"ENG-28816".to_string())
                        || stack.len() == 1,
                    "stack should contain parent if cache knows it: {stack:?}",
                );
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
        // Two-char ASCII labels — guaranteed monospace alignment.
        assert_eq!(state_glyph("started"), "ip");
        assert_eq!(state_glyph("completed"), "dn");
        assert_eq!(state_glyph("canceled"), "cx");
        assert_eq!(state_glyph("unstarted"), "td");
        assert_eq!(state_glyph("backlog"), "bk");
        assert_eq!(state_glyph("triage"), "tr");
        assert_eq!(state_glyph("unknown"), "··");
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
        // M (modify slug, Phase 4a) still unwired
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('M')));
        assert!(app.toast.is_some());
        let toast = app.toast.as_ref().unwrap();
        assert!(toast.contains("not yet wired"));
        assert!(toast.contains("M"));

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
