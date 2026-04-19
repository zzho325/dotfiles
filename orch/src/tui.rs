use std::{
    collections::{HashMap, HashSet},
    io::{self, stdout},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
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
    widgets::Paragraph,
    Terminal,
};

use crate::state::{
    self, Task, TaskStatus, load_order,
    load_task_meta, load_tmux_sessions, save_order,
};

const FAST_TICK: Duration = Duration::from_secs(2);

// Rosé Pine Dawn palette

const TEXT: Color = Color::Rgb(0x57, 0x52, 0x79);     // #575279
const SUBTLE: Color = Color::Rgb(0x79, 0x75, 0x93);   // #797593
const MUTED: Color = Color::Rgb(0x98, 0x93, 0xa5);    // #9893a5
const LOVE: Color = Color::Rgb(0xb4, 0x63, 0x7a);     // #b4637a
const GOLD: Color = Color::Rgb(0xea, 0x9d, 0x34);     // #ea9d34
const PINE: Color = Color::Rgb(0x28, 0x69, 0x83);     // #286983
const FOAM: Color = Color::Rgb(0x56, 0x94, 0x9f);     // #56949f
const IRIS: Color = Color::Rgb(0x90, 0x7a, 0xa9);     // #907aa9
const HL_LOW: Color = Color::Rgb(0xf4, 0xed, 0xe8);   // #f4ede8

/// An item in the flat navigation list.
#[derive(Debug, Clone)]
enum ListItem {
    Task(usize),
    Pr(usize, usize), // (task_index, pr_index)
}

#[derive(Debug, Clone, PartialEq)]
enum Focus {
    List,
    Output,
    MessageInput,
}

/// Output pane state
struct OutputPane {
    run_id: String,
    lines: Vec<String>,
    scroll: usize,
    finished: bool,
    /// Cached file length for change detection
    last_len: u64,
}

struct App {
    tasks: Vec<Task>,
    order: Vec<String>,
    /// Which tasks have PRs expanded
    expanded: HashSet<usize>,
    /// Flat list of visible items (rebuilt on expand/fold)
    visible: Vec<ListItem>,
    /// Cursor into `visible`
    cursor: usize,
    last_fast: Instant,
    should_quit: bool,
    /// Output pane (if open)
    output: Option<OutputPane>,
    focus: Focus,
    /// Run IDs the user has dismissed
    read_runs: HashSet<String>,
    /// Last known run count (to detect new runs)
    last_run_count: usize,
    /// Message input buffer
    message_input: String,
    /// Skip file I/O (for tests)
    readonly: bool,
    /// Whether daemon is providing cache
    daemon_alive: bool,
}

impl App {
    fn new() -> Self {
        let tasks = Self::load_from_cache();
        let order: Vec<String> =
            tasks.iter().map(|t| t.name.clone()).collect();
        let visible = Self::build_visible(&tasks, &HashSet::new());
        let last_run_count = crate::runs::list_runs(100).len();
        let daemon_alive = crate::cache::is_daemon_alive();

        let app = Self {
            tasks,
            order,
            expanded: HashSet::new(),
            visible,
            cursor: 0,
            last_fast: Instant::now(),
            should_quit: false,
            output: None,
            focus: Focus::List,
            read_runs: HashSet::new(),
            last_run_count,
            message_input: String::new(),
            readonly: false,
            daemon_alive,
        };
        app.sync_tmux_numbers();
        app
    }

    /// Load tasks with status and PR data. Prefers daemon cache;
    /// falls back to live tmux poll (no PR data) if daemon is dead.
    fn load_from_cache() -> Vec<Task> {
        let order = load_order();
        let status_cache = crate::cache::read_status();
        let pr_cache = crate::cache::read_prs();
        let daemon_alive = crate::cache::is_daemon_alive();

        // Live tmux poll used only when the daemon is not
        // writing fresh status.
        let live_sessions = if daemon_alive {
            None
        } else {
            Some(load_tmux_sessions())
        };

        state::ordered_task_names(&order)
            .into_iter()
            .map(|name| {
                let meta = load_task_meta(&name);

                let status = if daemon_alive {
                    status_cache
                        .tasks
                        .get(&name)
                        .map(|ct| match ct.status.as_str() {
                            "ready" => TaskStatus::Ready,
                            "working" => TaskStatus::Working,
                            "input" => TaskStatus::Input,
                            "attached" => TaskStatus::Attached,
                            "paused" => TaskStatus::Paused,
                            _ => TaskStatus::Idle,
                        })
                        .unwrap_or(TaskStatus::Idle)
                } else if let Some(sessions) = &live_sessions {
                    // No prev_hashes without the daemon — worker
                    // shows as Working while alive, Idle/Paused
                    // otherwise. Good enough for the rare case
                    // of running TUI without daemon.
                    state::derive_status(&meta, sessions, &HashMap::new())
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

                Task { name, meta, status, prs }
            })
            .collect()
    }

    fn build_visible(
        tasks: &[Task],
        expanded: &HashSet<usize>,
    ) -> Vec<ListItem> {
        let mut items = Vec::new();
        for (i, task) in tasks.iter().enumerate() {
            items.push(ListItem::Task(i));
            if expanded.contains(&i) {
                for j in 0..task.prs.len() {
                    items.push(ListItem::Pr(i, j));
                }
            }
        }
        items
    }

    fn rebuild_visible(&mut self) {
        self.visible =
            Self::build_visible(&self.tasks, &self.expanded);
        if self.cursor >= self.visible.len() {
            self.cursor =
                self.visible.len().saturating_sub(1);
        }
    }

    /// Get the task index for the currently selected item.
    fn selected_task_idx(&self) -> Option<usize> {
        match self.visible.get(self.cursor)? {
            ListItem::Task(i) => Some(*i),
            ListItem::Pr(i, _) => Some(*i),
        }
    }

    /// Count unread runs.
    /// Open output pane for the latest unread run.
    fn open_run(&mut self, run: &crate::runs::RunMeta) {
        let content = crate::runs::read_output(&run.id);
        let last_len = content.len() as u64;
        let lines: Vec<String> =
            content.lines().map(String::from).collect();
        self.output = Some(OutputPane {
            run_id: run.id.clone(),
            lines,
            scroll: 0,
            finished: run.finished_at.is_some(),
            last_len,
        });
        self.focus = Focus::Output;
    }

    fn open_latest_run(&mut self) {
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

    fn refresh_output(&mut self) {
        if let Some(pane) = &mut self.output {
            // Skip re-read if file size unchanged
            let cur_len = crate::runs::output_len(&pane.run_id);
            if cur_len == pane.last_len {
                return;
            }
            pane.last_len = cur_len;

            let content = crate::runs::read_output(&pane.run_id);
            let new_lines: Vec<String> =
                content.lines().map(String::from).collect();
            let was_at_bottom =
                pane.scroll + 20 >= pane.lines.len();
            pane.lines = new_lines;
            if was_at_bottom {
                pane.scroll =
                    pane.lines.len().saturating_sub(1);
            }
        }
    }

    fn dismiss_output(&mut self) {
        if let Some(pane) = self.output.take() {
            self.read_runs.insert(pane.run_id);
        }
        self.focus = Focus::List;
    }

    fn check_new_runs(&mut self) {
        let runs = crate::runs::list_runs(50);
        let current_count = runs.len();
        if current_count > self.last_run_count
            && self.output.is_none()
        {
            if let Some(newest) = runs.first().cloned() {
                self.open_run(&newest);
            }
        }
        self.last_run_count = current_count;
    }

    fn refresh_fast(&mut self) {
        self.daemon_alive = crate::cache::is_daemon_alive();
        self.tasks = Self::load_from_cache();

        // Save order only when tasks are added or removed
        let task_count = self.tasks.len();
        if task_count != self.order.len() {
            self.order =
                self.tasks.iter().map(|t| t.name.clone()).collect();
            if !self.readonly { save_order(&self.order); }
        }

        self.rebuild_visible();
        self.sync_tmux_numbers();
        self.last_fast = Instant::now();
    }

    fn selected_task(&self) -> Option<&Task> {
        let idx = self.selected_task_idx()?;
        self.tasks.get(idx)
    }

    fn move_up(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    fn move_down(&mut self) {
        if self.cursor + 1 < self.visible.len() {
            self.cursor += 1;
        }
    }

    fn expand(&mut self) {
        if let Some(ListItem::Task(i)) = self.visible.get(self.cursor) {
            if !self.tasks[*i].prs.is_empty()
                && !self.expanded.contains(i)
            {
                self.expanded.insert(*i);
                self.rebuild_visible();
            }
        }
    }

    fn fold(&mut self) {
        let task_idx = match self.visible.get(self.cursor) {
            Some(ListItem::Task(i)) => *i,
            Some(ListItem::Pr(i, _)) => *i,
            None => return,
        };
        if self.expanded.remove(&task_idx) {
            self.rebuild_visible();
            // Move cursor to the parent task
            self.cursor = self
                .visible
                .iter()
                .position(|v| matches!(v, ListItem::Task(i) if *i == task_idx))
                .unwrap_or(self.cursor);
        }
    }

    fn reorder(&mut self, delta: isize) {
        let Some(ListItem::Task(idx)) = self.visible.get(self.cursor)
        else {
            return;
        };
        let idx = *idx;
        let target = (idx as isize) + delta;
        if target < 0 || target as usize >= self.tasks.len() {
            return;
        }
        let target = target as usize;
        self.tasks.swap(idx, target);
        let had_a = self.expanded.remove(&idx);
        let had_b = self.expanded.remove(&target);
        if had_a { self.expanded.insert(target); }
        if had_b { self.expanded.insert(idx); }
        self.order =
            self.tasks.iter().map(|t| t.name.clone()).collect();
        if !self.readonly { save_order(&self.order); }
        self.rebuild_visible();
        self.cursor = self
            .visible
            .iter()
            .position(
                |v| matches!(v, ListItem::Task(i) if *i == target),
            )
            .unwrap_or(self.cursor);
        self.sync_tmux_numbers();
    }

    fn sync_tmux_numbers(&self) {
        if self.readonly { return; }
        let output = Command::new("tmux")
            .args(["list-sessions", "-F", "#{session_name}"])
            .stderr(Stdio::null())
            .output()
            .ok();
        let Some(output) = output.filter(|o| o.status.success())
        else {
            return;
        };
        let mut names: Vec<String> =
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(String::from)
                .collect();

        for (i, task) in self.tasks.iter().enumerate() {
            let session = &task.meta.session;
            if session.is_empty() {
                continue;
            }
            let new_name = format!("{}-{session}", i + 1);
            let pos = names.iter().position(|n| {
                state::session_matches(n, session)
            });
            if let Some(pos) = pos {
                let current = names[pos].clone();
                if current != new_name {
                    let _ = Command::new("tmux")
                        .args([
                            "rename-session",
                            "-t", &current,
                            &new_name,
                        ])
                        .stderr(Stdio::null())
                        .status();
                }
                names.remove(pos);
            }
        }
    }

    fn jump_to_session(&self) {
        let Some(task) = self.selected_task() else {
            return;
        };
        let session = &task.meta.session;
        if session.is_empty() {
            return;
        }

        let sessions = load_tmux_sessions();
        let actual_name = sessions
            .values()
            .find(|s| state::session_matches(&s.name, session))
            .map(|s| s.name.clone())
            .unwrap_or_else(|| session.clone());

        let in_tmux = std::env::var("TMUX").is_ok();
        let action = if in_tmux {
            "switch-client"
        } else {
            "attach-session"
        };
        let _ = Command::new("tmux")
            .args([action, "-t", &actual_name])
            .status();
    }

    fn selected_pr(&self) -> Option<&state::PrData> {
        match self.visible.get(self.cursor)? {
            ListItem::Pr(ti, pi) => {
                self.tasks[*ti].prs.get(*pi)
            }
            ListItem::Task(ti) => {
                self.tasks[*ti].prs.first()
            }
        }
    }

    fn open_pr_browser(&self) {
        let Some(pr) = self.selected_pr() else {
            return;
        };
        if pr.number == 0 {
            return;
        }
        let _ = Command::new("open")
            .arg(format!(
                "https://github.com/column/column/pull/{}",
                pr.number,
            ))
            .stderr(Stdio::null())
            .stdout(Stdio::null())
            .status();
    }
}

// Rendering

fn status_str(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Ready => "ready",
        TaskStatus::Working => "working",
        TaskStatus::Input => "input",
        TaskStatus::Idle => "idle",
        TaskStatus::Paused => "paused",
        TaskStatus::Attached => "·",
    }
}

fn status_color(status: TaskStatus) -> Color {
    match status {
        TaskStatus::Ready => PINE,
        TaskStatus::Working => FOAM,
        TaskStatus::Input => GOLD,
        TaskStatus::Paused => IRIS,
        TaskStatus::Idle | TaskStatus::Attached => MUTED,
    }
}

/// Compute column positions relative to terminal width.
/// Dots are anchored from the right edge so they don't get cut off.
struct Cols {
    status_end: usize,
    ci: usize,
    approve: usize,
    codex: usize,
}

impl Cols {
    fn for_width(w: usize) -> Self {
        // Dots at right edge: codex at w-4, approve at w-8, ci at w-12
        // Status label ends just before CI column
        let codex = w.saturating_sub(4);
        let approve = w.saturating_sub(8);
        let ci = w.saturating_sub(12);
        let status_end = ci.saturating_sub(3);
        Self { status_end, ci, approve, codex }
    }
}

/// Build a fixed-width line as a char buffer, then convert to styled spans.
fn fixed_line(inner_w: usize) -> Vec<char> {
    vec![' '; inner_w]
}

fn place(buf: &mut [char], pos: usize, s: &str) {
    for (i, ch) in s.chars().enumerate() {
        let p = pos + i;
        if p < buf.len() {
            buf[p] = ch;
        }
    }
}


fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();

    // Use a Block for the rounded border
    let block = ratatui::widgets::Block::default()
        .borders(ratatui::widgets::Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(MUTED));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let w = inner.width as usize;
    let cols = Cols::for_width(w);
    let mut lines: Vec<Line> = Vec::new();

    // Header: title + column headers on one line, using char buffer
    let task_count = app.tasks.len();
    let pr_count: usize = app.tasks.iter().map(|t| t.prs.len()).sum();
    let has_prs = app.tasks.iter().any(|t| !t.prs.is_empty());

    let mut hdr = fixed_line(w);
    let title = if pr_count > 0 {
        format!(" orch  {task_count} tasks · {pr_count} PRs")
    } else {
        format!(" orch  {task_count} tasks")
    };
    place(&mut hdr, 0, &title);

    // Build header using same char buffer + span pattern as PR rows
    if has_prs && w > cols.codex {
        hdr[cols.ci] = 'c';
        hdr[cols.approve] = 'a';
        hdr[cols.codex] = 'x';
    }
    {
        // Split: " orch" (bold) + rest-of-title (muted) + dots area
        let prefix: String = hdr[..cols.ci.min(w)].iter().collect();
        let mut spans = Vec::new();

        // Bold "orch" within the prefix
        if prefix.len() >= 5 {
            spans.push(Span::styled(
                prefix[..5].to_string(),
                Style::default().fg(TEXT).bold(),
            ));
            spans.push(Span::styled(
                prefix[5..].to_string(),
                Style::default().fg(MUTED),
            ));
        } else {
            spans.push(Span::styled(prefix, Style::default().fg(MUTED)));
        }

        if has_prs && w > cols.codex {
            spans.push(Span::styled("c", Style::default().fg(MUTED)));
            let gap1: String =
                hdr[cols.ci + 1..cols.approve].iter().collect();
            spans.push(Span::raw(gap1));
            spans.push(Span::styled("a", Style::default().fg(MUTED)));
            let gap2: String =
                hdr[cols.approve + 1..cols.codex].iter().collect();
            spans.push(Span::raw(gap2));
            spans.push(Span::styled("x", Style::default().fg(MUTED)));
            if cols.codex + 1 < w {
                let tail: String =
                    hdr[cols.codex + 1..].iter().collect();
                spans.push(Span::raw(tail));
            }
        }
        lines.push(Line::from(spans));
    }

    // Render visible items (flat list of tasks + expanded PRs)
    for (vi, item) in app.visible.iter().enumerate() {
        let sel = vi == app.cursor;

        match item {
            ListItem::Task(ti) => {
                let task = &app.tasks[*ti];
                let label = status_str(task.status);
                let label_start =
                    (cols.status_end + 1).saturating_sub(label.len());

                let is_expanded = app.expanded.contains(ti);
                let has_prs_for_task = !task.prs.is_empty();
                let gutter = if sel && has_prs_for_task {
                    if is_expanded { "▾ " } else { "▸ " }
                } else if sel {
                    "▸ "
                } else if has_prs_for_task {
                    if is_expanded { "▾ " } else { "▹ " }
                } else {
                    "  "
                };

                // PR badge
                let badge = if has_prs_for_task && !is_expanded {
                    format!(" [{}]", task.prs.len())
                } else {
                    String::new()
                };

                let max_name = label_start
                    .saturating_sub(3 + badge.chars().count());
                let name_trunc = truncate(&task.name, max_name);
                let name_w =
                    name_trunc.chars().count() + badge.chars().count();
                let pad1 = label_start.saturating_sub(2 + name_w);
                let tail_len = w.saturating_sub(cols.status_end + 1);

                let gutter_style = if sel {
                    Style::default().fg(IRIS)
                } else {
                    Style::default().fg(MUTED)
                };
                let name_style = if sel {
                    Style::default().fg(TEXT).bold()
                } else {
                    Style::default().fg(TEXT)
                };

                let mut line = Line::from(vec![
                    Span::styled(gutter, gutter_style),
                    Span::styled(name_trunc, name_style),
                    Span::styled(badge, Style::default().fg(MUTED)),
                    Span::raw(" ".repeat(pad1)),
                    Span::styled(
                        label,
                        Style::default().fg(status_color(task.status)),
                    ),
                    Span::raw(" ".repeat(tail_len)),
                ]);

                if sel {
                    line = line.style(Style::default().bg(HL_LOW));
                }
                lines.push(line);
            }
            ListItem::Pr(ti, pi) => {
                let pr = &app.tasks[*ti].prs[*pi];
                let mut buf = fixed_line(w);
                let gutter = if sel { "  ▸ " } else { "    " };
                let num_str = format!("#{} ", pr.number);
                let max_title = cols.ci.saturating_sub(5 + num_str.len());
                let text = format!(
                    "{}{}",
                    num_str,
                    truncate(&pr.title, max_title),
                );
                place(&mut buf, 0, gutter);
                place(&mut buf, 4, &text);

                let ci_ch = match pr.ci_pass {
                    Some(true) => '·',
                    Some(false) => '✗',
                    None => '○',
                };
                let ap_ch = if pr.approved { '·' } else { '○' };
                let cx_ch = match pr.codex {
                    state::CodexStatus::ThumbsUp => '·',
                    state::CodexStatus::Commented => '△',
                    state::CodexStatus::None => ' ',
                };

                if w > cols.ci { buf[cols.ci] = ci_ch; }
                if w > cols.approve { buf[cols.approve] = ap_ch; }
                if w > cols.codex { buf[cols.codex] = cx_ch; }

                let prefix: String =
                    buf[..cols.ci.min(w)].iter().collect();
                let mut spans = vec![Span::styled(
                    prefix,
                    if sel {
                        Style::default().fg(TEXT)
                    } else {
                        Style::default().fg(SUBTLE)
                    },
                )];

                if w > cols.ci {
                    let ci_style = match pr.ci_pass {
                        Some(true) => Style::default().fg(PINE),
                        Some(false) => Style::default().fg(LOVE),
                        None => Style::default().fg(MUTED),
                    };
                    spans.push(Span::styled(
                        ci_ch.to_string(), ci_style,
                    ));
                    let gap: String =
                        buf[cols.ci + 1..cols.approve.min(w)]
                            .iter().collect();
                    spans.push(Span::raw(gap));
                }
                if w > cols.approve {
                    let ap_style = if pr.approved {
                        Style::default().fg(PINE)
                    } else {
                        Style::default().fg(MUTED)
                    };
                    spans.push(Span::styled(
                        ap_ch.to_string(), ap_style,
                    ));
                    let gap: String =
                        buf[cols.approve + 1..cols.codex.min(w)]
                            .iter().collect();
                    spans.push(Span::raw(gap));
                }
                if w > cols.codex {
                    let cx_style = match pr.codex {
                        state::CodexStatus::ThumbsUp => {
                            Style::default().fg(PINE)
                        }
                        state::CodexStatus::Commented => {
                            Style::default().fg(GOLD)
                        }
                        state::CodexStatus::None => {
                            Style::default().fg(MUTED)
                        }
                    };
                    spans.push(Span::styled(
                        cx_ch.to_string(), cx_style,
                    ));
                    if cols.codex + 1 < w {
                        let tail: String =
                            buf[cols.codex + 1..].iter().collect();
                        spans.push(Span::raw(tail));
                    }
                }

                let mut line = Line::from(spans);
                if sel {
                    line = line.style(Style::default().bg(HL_LOW));
                }
                lines.push(line);
            }
        }
    }

    if app.tasks.is_empty() {
        lines.push(Line::styled(
            "  no tasks",
            Style::default().fg(MUTED),
        ));
    }

    // Split layout if output pane is open
    if let Some(pane) = &app.output {
        let chunks = ratatui::layout::Layout::vertical([
            ratatui::layout::Constraint::Percentage(50),
            ratatui::layout::Constraint::Length(1), // divider
            ratatui::layout::Constraint::Min(5),    // output
        ])
        .split(inner);

        // Task list (top)
        let paragraph = Paragraph::new(lines);
        frame.render_widget(paragraph, chunks[0]);

        // Divider
        let status = if pane.finished { "done" } else { "running…" };
        let divider_text = format!(
            "─ Output ─── {status} ───"
        );
        let pad = chunks[1].width as usize
            - divider_text.len().min(chunks[1].width as usize);
        let divider = Line::from(vec![
            Span::styled(divider_text, Style::default().fg(MUTED)),
            Span::styled(
                "─".repeat(pad),
                Style::default().fg(MUTED),
            ),
        ]);
        frame.render_widget(Paragraph::new(vec![divider]), chunks[1]);

        // Output content (bottom) with word wrap and scroll
        let output_lines: Vec<Line> = pane.lines
            .iter()
            .map(|l| Line::styled(l.as_str(), Style::default().fg(SUBTLE)))
            .collect();
        frame.render_widget(
            Paragraph::new(output_lines)
                .wrap(ratatui::widgets::Wrap { trim: false })
                .scroll((pane.scroll as u16, 0)),
            chunks[2],
        );
    } else {
        // No output pane — task list fills the whole area
        let paragraph = Paragraph::new(lines);
        frame.render_widget(paragraph, inner);
    }

    // Message input line at the very bottom (inside border)
    if app.focus == Focus::MessageInput {
        let input_area = Rect {
            x: inner.x,
            y: inner.y + inner.height.saturating_sub(1),
            width: inner.width,
            height: 1,
        };
        let input_line = Line::from(vec![
            Span::styled("msg", Style::default().fg(IRIS)),
            Span::styled("▸ ", Style::default().fg(MUTED)),
            Span::styled(
                app.message_input.as_str(),
                Style::default().fg(TEXT),
            ),
            Span::styled("_", Style::default().fg(IRIS)),
        ]);
        frame.render_widget(
            Paragraph::new(vec![input_line])
                .style(Style::default().bg(HL_LOW)),
            input_area,
        );
    }
}

fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if s.chars().count() <= max {
        return s.to_string();
    }
    let t: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{t}…")
}

// Terminal lifecycle

fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = stdout().execute(LeaveAlternateScreen);
        original(info);
    }));
}

pub fn run() -> io::Result<()> {
    install_panic_hook();
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;
    let mut app = App::new();

    loop {
        terminal.draw(|f| render(f, &app))?;

        if event::poll(FAST_TICK)? {
            if let Event::Key(key) = event::read()? {
                match handle_key(&mut app, key) {
                    Action::Continue => {}
                    Action::Quit => break,
                    Action::Suspend(f) => {
                        disable_raw_mode()?;
                        stdout().execute(LeaveAlternateScreen)?;
                        f(&app);
                        enable_raw_mode()?;
                        stdout().execute(EnterAlternateScreen)?;
                        terminal = Terminal::new(
                            CrosstermBackend::new(stdout()),
                        )?;
                    }
                }
            }
        }

        if app.last_fast.elapsed() >= FAST_TICK {
            app.refresh_fast();
            app.check_new_runs();
            app.refresh_output();
        }
        if app.should_quit {
            break;
        }
    }

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}

enum Action {
    Continue,
    Quit,
    Suspend(Box<dyn FnOnce(&App)>),
}

fn handle_key(app: &mut App, key: KeyEvent) -> Action {
    // Message input mode
    if app.focus == Focus::MessageInput {
        match key.code {
            KeyCode::Enter => {
                if !app.message_input.trim().is_empty() {
                    crate::write_inbox(&app.message_input);
                }
                app.message_input.clear();
                app.focus = Focus::List;
                return Action::Continue;
            }
            KeyCode::Esc => {
                app.message_input.clear();
                app.focus = Focus::List;
                return Action::Continue;
            }
            KeyCode::Backspace => {
                app.message_input.pop();
                return Action::Continue;
            }
            KeyCode::Char(c) => {
                app.message_input.push(c);
                return Action::Continue;
            }
            _ => return Action::Continue,
        }
    }

    // Output pane focused
    if app.focus == Focus::Output && app.output.is_some() {
        match (key.modifiers, key.code) {
            (_, KeyCode::Esc) | (_, KeyCode::Char('q')) => {
                app.dismiss_output();
                Action::Continue
            }
            (_, KeyCode::Char('j')) | (_, KeyCode::Down) => {
                if let Some(pane) = &mut app.output {
                    if pane.scroll + 1 < pane.lines.len() {
                        pane.scroll += 1;
                    }
                }
                Action::Continue
            }
            (_, KeyCode::Char('k')) | (_, KeyCode::Up) => {
                if let Some(pane) = &mut app.output {
                    pane.scroll = pane.scroll.saturating_sub(1);
                }
                Action::Continue
            }
            (_, KeyCode::Char('G')) => {
                if let Some(pane) = &mut app.output {
                    pane.scroll =
                        pane.lines.len().saturating_sub(1);
                }
                Action::Continue
            }
            (_, KeyCode::Char('g')) => {
                if let Some(pane) = &mut app.output {
                    pane.scroll = 0;
                }
                Action::Continue
            }
            (_, KeyCode::Tab) => {
                app.focus = Focus::List;
                Action::Continue
            }
            _ => Action::Continue,
        }
    } else {
        // List focused (default)
        match (key.modifiers, key.code) {
            (_, KeyCode::Char('q')) => {
                if app.output.is_some() {
                    app.dismiss_output();
                    Action::Continue
                } else {
                    Action::Quit
                }
            }
            (KeyModifiers::CONTROL, KeyCode::Char('c')) => Action::Quit,
            (_, KeyCode::Esc) => {
                if app.output.is_some() {
                    app.dismiss_output();
                }
                Action::Continue
            }
            (_, KeyCode::Char('j')) => {
                app.move_down();
                Action::Continue
            }
            (_, KeyCode::Char('k')) => {
                app.move_up();
                Action::Continue
            }
            (_, KeyCode::Char('l')) => {
                app.expand();
                Action::Continue
            }
            (_, KeyCode::Char('h')) => {
                app.fold();
                Action::Continue
            }
            (_, KeyCode::Char('J')) => {
                app.reorder(1);
                Action::Continue
            }
            (_, KeyCode::Char('K')) => {
                app.reorder(-1);
                Action::Continue
            }
            (_, KeyCode::Char('r')) => {
                app.open_latest_run();
                Action::Continue
            }
            (_, KeyCode::Char('m')) => {
                app.focus = Focus::MessageInput;
                app.message_input.clear();
                Action::Continue
            }
            (_, KeyCode::Tab) => {
                if app.output.is_some() {
                    app.focus = Focus::Output;
                }
                Action::Continue
            }
            (_, KeyCode::Enter) => {
                match app.visible.get(app.cursor) {
                    Some(ListItem::Task(_)) => {
                        Action::Suspend(Box::new(|a| {
                            a.jump_to_session()
                        }))
                    }
                    Some(ListItem::Pr(_, _)) => {
                        app.open_pr_browser();
                        Action::Continue
                    }
                    None => Action::Continue,
                }
            }
            (_, KeyCode::Char('o')) => {
                app.open_pr_browser();
                Action::Continue
            }
            _ => Action::Continue,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    fn test_tasks() -> Vec<Task> {
        vec![
            Task {
                name: "migration-safety".into(),
                meta: state::TaskMeta {
                    session: "task-migration-safety".into(),
                    prs: vec![25810, 25812],
                    ..Default::default()
                },
                status: TaskStatus::Ready,
                prs: vec![
                    state::PrData {
                        number: 25810,
                        title: "safe migration helper".into(),
                        ci_pass: Some(true),
                        approved: true,
                        codex: state::CodexStatus::Commented,
                    },
                    state::PrData {
                        number: 25812,
                        title: "add lock_wait_timeout".into(),
                        ci_pass: Some(true),
                        approved: false,
                        codex: state::CodexStatus::Commented,
                    },
                ],
            },
            Task {
                name: "ach-batch-nacha".into(),
                meta: state::TaskMeta {
                    session: "task-ach-batch-nacha".into(),
                    ..Default::default()
                },
                status: TaskStatus::Working,
                prs: vec![],
            },
            Task {
                name: "slow-api".into(),
                meta: state::TaskMeta {
                    session: "task-slow-api".into(),
                    ..Default::default()
                },
                status: TaskStatus::Idle,
                prs: vec![],
            },
        ]
    }

    fn make_test_app(
        tasks: Vec<Task>,
        cursor: usize,
        output: Option<OutputPane>,
    ) -> App {
        let mut expanded = HashSet::new();
        for (i, t) in tasks.iter().enumerate() {
            if !t.prs.is_empty() {
                expanded.insert(i);
            }
        }
        let visible =
            App::build_visible(&tasks, &expanded);
        let focus = if output.is_some() {
            Focus::Output
        } else {
            Focus::List
        };
        App {
            tasks,
            order: Vec::new(),
            expanded,
            visible,
            cursor,
            last_fast: Instant::now(),
            should_quit: false,
            output,
            focus,
            read_runs: HashSet::new(),
            last_run_count: 0,
            message_input: String::new(),
            readonly: true,
            daemon_alive: true,
        }
    }

    fn render_app(
        app: &App,
        width: u16,
        height: u16,
    ) -> String {
        let mut terminal =
            Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|f| render(f, &app))
            .unwrap();
        format!("{}", terminal.backend())
    }

    fn render_to_string(
        tasks: Vec<Task>,
        cursor: usize,
        width: u16,
        height: u16,
    ) -> String {
        let app = make_test_app(tasks, cursor, None);
        render_app(&app, width, height)
    }

    #[test]
    fn layout_selected_first() {
        let output = render_to_string(test_tasks(), 0, 60, 10);
        assert!(output.contains("orch"));
        assert!(output.contains("3 tasks"));
        assert!(output.contains("2 PRs"));
        // Selected + expanded = ▾
        assert!(output.contains("▾ migration-safety"));
        assert!(output.contains("ready"));
        assert!(output.contains("working"));
        assert!(output.contains("idle"));
        assert!(output.contains("#25810"));
        assert!(output.contains("#25812"));
    }

    #[test]
    fn layout_selected_second() {
        // cursor=1 is the first PR row (since task 0 is expanded)
        let output = render_to_string(test_tasks(), 1, 60, 10);
        assert!(output.contains("#25810"));
    }

    #[test]
    fn layout_no_prs() {
        let tasks = vec![Task {
            name: "simple-task".into(),
            meta: state::TaskMeta::default(),
            status: TaskStatus::Working,
            prs: vec![],
        }];
        let output = render_to_string(tasks, 0, 60, 5);
        assert!(output.contains("▸ simple-task"));
        assert!(output.contains("working"));
        // No column headers when no PRs
        assert!(!output.contains("⚙"));
    }

    fn real_tasks() -> Vec<Task> {
        vec![
            Task {
                name: "agentserver".into(),
                meta: state::TaskMeta {
                    session: "task-agentserver".into(),
                    prs: vec![25827],
                    ..Default::default()
                },
                status: TaskStatus::Input,
                prs: vec![state::PrData {
                    number: 25827,
                    title: "feat(remoteagent): live tool progress in Slack".into(),
                    ci_pass: Some(false),
                    approved: false,
                    codex: state::CodexStatus::Commented,
                }],
            },
            Task {
                name: "check-issuance-scope".into(),
                meta: state::TaskMeta::default(),
                status: TaskStatus::Ready,
                prs: vec![],
            },
            Task {
                name: "ach-batch-followup".into(),
                meta: state::TaskMeta::default(),
                status: TaskStatus::Ready,
                prs: vec![],
            },
            Task {
                name: "ach-batch-nacha".into(),
                meta: state::TaskMeta::default(),
                status: TaskStatus::Working,
                prs: vec![],
            },
            Task {
                name: "migration-safety".into(),
                meta: state::TaskMeta::default(),
                status: TaskStatus::Ready,
                prs: vec![],
            },
            Task {
                name: "slow-internal-api".into(),
                meta: state::TaskMeta::default(),
                status: TaskStatus::Ready,
                prs: vec![],
            },
            Task {
                name: "temporal-validate-records".into(),
                meta: state::TaskMeta::default(),
                status: TaskStatus::Ready,
                prs: vec![],
            },
        ]
    }

    #[test]
    fn status_column_stable_across_all_selections() {
        let tasks = real_tasks();

        fn find_status_col(line: &str) -> Option<usize> {
            let chars: Vec<char> = line.chars().collect();
            for word in ["ready", "working", "input", "idle"] {
                // Find the word by char position (not byte)
                let wchars: Vec<char> = word.chars().collect();
                for i in 0..chars.len().saturating_sub(wchars.len()) {
                    if chars[i..i + wchars.len()] == wchars[..] {
                        return Some(i + wchars.len()); // end col
                    }
                }
            }
            None
        }

        // Render with each task selected
        let outputs: Vec<String> = (0..tasks.len())
            .map(|sel| render_to_string(tasks.clone(), sel, 80, 14))
            .collect();

        // Collect ALL status end-columns across ALL lines in ALL renders
        let mut all_end_cols: Vec<(usize, String, usize)> = Vec::new();
        for (sel, output) in outputs.iter().enumerate() {
            for line in output.lines() {
                // Skip header and PR lines
                if line.contains("orch") || line.contains('#') {
                    continue;
                }
                if let Some(end_col) = find_status_col(line) {
                    let name = line.trim().split_whitespace().next()
                        .unwrap_or("?").to_string();
                    all_end_cols.push((sel, name, end_col));
                }
            }
        }

        // All status labels must END at the same char column
        if let Some(first) = all_end_cols.first() {
            let expected = first.2;
            for (sel, name, col) in &all_end_cols {
                assert_eq!(
                    *col, expected,
                    "status end-col={col} for '{name}' (sel={sel}), \
                     expected {expected}. This means the status label \
                     is jumping."
                );
            }
        }
    }

    /// Find the char-column (not byte-index) of `needle` in `s`,
    /// scanning from the right.
    fn rfind_char_col(s: &str, needle: char) -> Option<usize> {
        let chars: Vec<char> = s.chars().collect();
        chars.iter().rposition(|&c| c == needle)
    }

    fn find_char_col(s: &str, needle: char) -> Option<usize> {
        s.chars().position(|c| c == needle)
    }

    #[test]
    fn dot_columns_aligned_with_headers() {
        let tasks = real_tasks();
        let output = render_to_string(tasks, 0, 80, 14);

        let header_line = output
            .lines()
            .find(|l| l.contains("orch") && l.contains(" c "))
            .expect("no header line");

        let pr_line = output
            .lines()
            .find(|l| l.contains("#25827"))
            .expect("no PR line");

        // CI: rightmost 'c' in header vs ✗ in PR
        let ci_hdr =
            rfind_char_col(header_line, 'c').unwrap();
        let ci_dot =
            find_char_col(pr_line, '✗').unwrap();
        assert_eq!(
            ci_dot, ci_hdr,
            "CI: dot={ci_dot} header={ci_hdr}"
        );

        // Approval: rightmost 'a' vs ○
        let ap_hdr =
            rfind_char_col(header_line, 'a').unwrap();
        let ap_dot =
            find_char_col(pr_line, '○').unwrap();
        assert_eq!(
            ap_dot, ap_hdr,
            "approval: dot={ap_dot} header={ap_hdr}"
        );

        // Codex: 'x' vs △ (commented status)
        let cx_hdr =
            rfind_char_col(header_line, 'x').unwrap();
        let cx_dot =
            find_char_col(pr_line, '△').unwrap();
        assert_eq!(
            cx_dot, cx_hdr,
            "codex: dot={cx_dot} header={cx_hdr}"
        );
    }

    #[test]
    fn pr_rows_only_under_task_with_prs() {
        let tasks = real_tasks();
        // Render with selection on different tasks
        for sel in 0..tasks.len() {
            let output = render_to_string(tasks.clone(), sel, 80, 14);
            // #25827 should always appear (it's under agentserver)
            assert!(
                output.contains("#25827"),
                "PR #25827 missing when sel={sel}"
            );
            // Count PR lines — should always be exactly 1
            let pr_lines = output
                .lines()
                .filter(|l| l.contains("#25827"))
                .count();
            assert_eq!(
                pr_lines, 1,
                "expected 1 PR line, got {pr_lines} when sel={sel}"
            );
        }
    }

    #[test]
    fn has_border() {
        let output = render_to_string(test_tasks(), 0, 70, 10);
        assert!(output.contains('╭'), "missing top-left border");
        assert!(output.contains('╰'), "missing bottom-left border");
    }

    #[test]
    fn snapshot_all_selections() {
        let tasks = real_tasks();
        for sel in 0..tasks.len() {
            let output = render_to_string(tasks.clone(), sel, 80, 14);
            eprintln!("\n=== sel={sel} ({}) ===\n{output}", tasks[sel].name);
        }
    }

    fn render_with_output(
        tasks: Vec<Task>,
        output_lines: Vec<&str>,
        width: u16,
        height: u16,
    ) -> String {
        let pane = OutputPane {
            run_id: "test-run".into(),
            lines: output_lines
                .iter()
                .map(|s| s.to_string())
                .collect(),
            scroll: 0,
            finished: true,
            last_len: 0,
        };
        let app = make_test_app(tasks, 0, Some(pane));
        render_app(&app, width, height)
    }

    // Unit tests for App logic (non-rendering)

    #[test]
    fn reorder_swaps_and_preserves_expanded() {
        let mut app = make_test_app(test_tasks(), 0, None);
        // Task 0 is expanded (has PRs)
        assert!(app.expanded.contains(&0));
        assert_eq!(app.tasks[0].name, "migration-safety");

        // Reorder down: migration-safety → position 1
        app.reorder(1);
        assert_eq!(app.tasks[0].name, "ach-batch-nacha");
        assert_eq!(app.tasks[1].name, "migration-safety");
        // Expanded set follows the task
        assert!(app.expanded.contains(&1));
        assert!(!app.expanded.contains(&0));
    }

    #[test]
    fn reorder_up_from_middle() {
        let mut app = make_test_app(test_tasks(), 0, None);
        // Fold so all items are tasks
        app.expanded.clear();
        app.rebuild_visible();
        // Move cursor to task 2 (slow-api)
        app.cursor = 2;
        assert_eq!(app.tasks[2].name, "slow-api");

        // Move up
        app.reorder(-1);
        assert_eq!(app.tasks[1].name, "slow-api");
        assert_eq!(app.tasks[2].name, "ach-batch-nacha");
    }

    #[test]
    fn reorder_noop_at_boundaries() {
        let mut app = make_test_app(test_tasks(), 0, None);
        let names_before: Vec<_> =
            app.tasks.iter().map(|t| t.name.clone()).collect();
        // Can't move first task up
        app.reorder(-1);
        let names_after: Vec<_> =
            app.tasks.iter().map(|t| t.name.clone()).collect();
        assert_eq!(names_before, names_after);

        // Can't move last task down
        app.expanded.clear();
        app.rebuild_visible();
        app.cursor = app.visible.len() - 1;
        app.reorder(1);
        let names_end: Vec<_> =
            app.tasks.iter().map(|t| t.name.clone()).collect();
        assert_eq!(names_before, names_end);
    }

    #[test]
    fn fold_expand_cycle() {
        let mut app = make_test_app(test_tasks(), 0, None);
        // Start expanded (make_test_app expands tasks with PRs)
        let initial_len = app.visible.len();
        assert!(initial_len > 3); // 3 tasks + 2 PRs = 5

        // Fold task 0
        app.fold();
        assert!(!app.expanded.contains(&0));
        assert_eq!(app.visible.len(), 3); // just tasks

        // Expand again
        app.expand();
        assert!(app.expanded.contains(&0));
        assert_eq!(app.visible.len(), initial_len);
    }

    #[test]
    fn cursor_clamps_after_fold() {
        let mut app = make_test_app(test_tasks(), 0, None);
        // Move cursor to PR row
        app.cursor = 2; // second PR
        assert!(
            matches!(app.visible[2], ListItem::Pr(0, 1))
        );

        // Fold — cursor should move to parent task
        app.fold();
        assert_eq!(app.cursor, 0);
        assert!(matches!(app.visible[0], ListItem::Task(0)));
    }

    #[test]
    fn truncate_handles_edge_cases() {
        assert_eq!(truncate("hello", 0), "");
        assert_eq!(truncate("hello", 5), "hello");
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 6), "hello…");
    }

    #[test]
    fn build_visible_with_no_expanded() {
        let tasks = test_tasks();
        let visible =
            App::build_visible(&tasks, &HashSet::new());
        // No expansions = just task items
        assert_eq!(visible.len(), 3);
        assert!(visible.iter().all(
            |v| matches!(v, ListItem::Task(_))
        ));
    }

    #[test]
    fn handle_key_message_input() {
        let mut app = make_test_app(test_tasks(), 0, None);
        app.focus = Focus::MessageInput;

        // Type characters
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE),
        );
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
        );
        assert_eq!(app.message_input, "hi");

        // Backspace
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
        );
        assert_eq!(app.message_input, "h");

        // Escape cancels
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        );
        assert_eq!(app.focus, Focus::List);
        assert!(app.message_input.is_empty());
    }

    // Snapshot tests for rendering edge cases

    #[test]
    fn narrow_terminal_no_panic() {
        // Very narrow terminal — should not panic
        let output = render_to_string(test_tasks(), 0, 30, 10);
        assert!(output.contains("orch"));
    }

    #[test]
    fn message_input_renders() {
        let mut app = make_test_app(test_tasks(), 0, None);
        app.focus = Focus::MessageInput;
        app.message_input = "hello orch".into();
        let output = render_app(&app, 70, 10);
        eprintln!("\n=== message input ===\n{output}");
        assert!(output.contains("msg"), "missing msg prompt");
        assert!(
            output.contains("hello orch"),
            "missing input text"
        );
    }

    #[test]
    fn codex_thumbsup_dot() {
        let tasks = vec![Task {
            name: "my-task".into(),
            meta: state::TaskMeta {
                prs: vec![100],
                ..Default::default()
            },
            status: TaskStatus::Ready,
            prs: vec![state::PrData {
                number: 100,
                title: "some pr".into(),
                ci_pass: Some(true),
                approved: true,
                codex: state::CodexStatus::ThumbsUp,
            }],
        }];
        let output = render_to_string(tasks, 0, 60, 6);
        // All three columns should show passing dots
        let pr_line = output
            .lines()
            .find(|l| l.contains("#100"))
            .expect("no PR line");
        // ci=·, approve=·, codex=· (all passing)
        let dots = pr_line.chars().filter(|&c| c == '·').count();
        assert!(dots >= 3, "expected 3 passing dots, got {dots}");
    }

    #[test]
    fn output_pane_renders() {
        let tasks = vec![
            Task {
                name: "agentserver".into(),
                meta: state::TaskMeta::default(),
                status: TaskStatus::Working,
                prs: vec![],
            },
            Task {
                name: "slow-api".into(),
                meta: state::TaskMeta::default(),
                status: TaskStatus::Ready,
                prs: vec![],
            },
        ];
        let output = render_with_output(
            tasks,
            vec![
                "Scanning workspace...",
                "> Found 2 active tasks",
                "> agentserver: working",
                "> slow-api: ready",
                "> Done.",
            ],
            70,
            16,
        );
        eprintln!("\n=== output pane ===\n{output}");
        // Should have the divider
        assert!(output.contains("Output"), "missing Output divider");
        assert!(output.contains("done"), "missing done status");
        // Should show output content
        assert!(
            output.contains("Scanning"),
            "missing output content"
        );
        // Should still show tasks
        assert!(
            output.contains("agentserver"),
            "missing task list"
        );
    }
}
