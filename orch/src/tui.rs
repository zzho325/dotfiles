use std::{
    collections::HashSet,
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

use crate::gh::PrCache;
use crate::state::{
    self, Task, TaskStatus, load_order, load_tasks,
    load_tmux_sessions, save_order,
};

const FAST_TICK: Duration = Duration::from_secs(2);
const SLOW_TICK: Duration = Duration::from_secs(30);

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
const _HL_MED: Color = Color::Rgb(0xdf, 0xda, 0xd9); // #dfdad9

/// An item in the flat navigation list.
#[derive(Debug, Clone)]
enum ListItem {
    Task(usize),
    Pr(usize, usize), // (task_index, pr_index)
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
    pr_cache: PrCache,
    last_fast: Instant,
    last_slow: Instant,
    should_quit: bool,
}

impl App {
    fn new() -> Self {
        let order = load_order();
        let sessions = load_tmux_sessions();
        let tasks = load_tasks(&order, &sessions);
        let pr_cache = PrCache::new();

        let all_prs: Vec<u32> = tasks
            .iter()
            .flat_map(|t| t.meta.prs.iter().copied())
            .collect();
        pr_cache.refresh(all_prs);

        let visible = Self::build_visible(&tasks, &HashSet::new());

        Self {
            tasks,
            order,
            expanded: HashSet::new(),
            visible,
            cursor: 0,
            pr_cache,
            last_fast: Instant::now(),
            last_slow: Instant::now(),
            should_quit: false,
        }
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

    fn refresh_fast(&mut self) {
        let sessions = load_tmux_sessions();
        self.tasks = load_tasks(&self.order, &sessions);

        for task in &mut self.tasks {
            task.prs = task
                .meta
                .prs
                .iter()
                .map(|&num| {
                    self.pr_cache
                        .get(num)
                        .unwrap_or(state::PrData {
                            number: num,
                            ..Default::default()
                        })
                })
                .collect();
        }

        self.rebuild_visible();
        self.last_fast = Instant::now();
    }

    fn refresh_slow(&mut self) {
        let all_prs: Vec<u32> = self
            .tasks
            .iter()
            .flat_map(|t| t.meta.prs.iter().copied())
            .collect();
        self.pr_cache.refresh(all_prs);
        self.last_slow = Instant::now();
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

    fn toggle_expand(&mut self) {
        let Some(item) = self.visible.get(self.cursor) else {
            return;
        };
        let task_idx = match item {
            ListItem::Task(i) => *i,
            ListItem::Pr(i, _) => *i, // fold parent
        };
        if self.tasks[task_idx].prs.is_empty() {
            return;
        }
        if self.expanded.contains(&task_idx) {
            self.expanded.remove(&task_idx);
            // Move cursor to parent task if on a PR
            if matches!(item, ListItem::Pr(_, _)) {
                // Find the parent in the new visible list
                self.rebuild_visible();
                self.cursor = self
                    .visible
                    .iter()
                    .position(|v| matches!(v, ListItem::Task(i) if *i == task_idx))
                    .unwrap_or(0);
                return;
            }
        } else {
            self.expanded.insert(task_idx);
        }
        self.rebuild_visible();
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

    fn reorder_up(&mut self) {
        let Some(ListItem::Task(idx)) = self.visible.get(self.cursor)
        else {
            return;
        };
        let idx = *idx;
        if idx > 0 {
            self.tasks.swap(idx, idx - 1);
            // Fix expanded set
            let had_a = self.expanded.remove(&idx);
            let had_b = self.expanded.remove(&(idx - 1));
            if had_a { self.expanded.insert(idx - 1); }
            if had_b { self.expanded.insert(idx); }
            self.order =
                self.tasks.iter().map(|t| t.name.clone()).collect();
            save_order(&self.order);
            self.rebuild_visible();
            // Move cursor to the new position
            self.cursor = self
                .visible
                .iter()
                .position(|v| matches!(v, ListItem::Task(i) if *i == idx - 1))
                .unwrap_or(self.cursor);
            self.sync_tmux_numbers();
        }
    }

    fn reorder_down(&mut self) {
        let Some(ListItem::Task(idx)) = self.visible.get(self.cursor)
        else {
            return;
        };
        let idx = *idx;
        if idx + 1 < self.tasks.len() {
            self.tasks.swap(idx, idx + 1);
            let had_a = self.expanded.remove(&idx);
            let had_b = self.expanded.remove(&(idx + 1));
            if had_a { self.expanded.insert(idx + 1); }
            if had_b { self.expanded.insert(idx); }
            self.order =
                self.tasks.iter().map(|t| t.name.clone()).collect();
            save_order(&self.order);
            self.rebuild_visible();
            self.cursor = self
                .visible
                .iter()
                .position(|v| matches!(v, ListItem::Task(i) if *i == idx + 1))
                .unwrap_or(self.cursor);
            self.sync_tmux_numbers();
        }
    }

    fn sync_tmux_numbers(&self) {
        let sessions = load_tmux_sessions();
        for (i, task) in self.tasks.iter().enumerate() {
            let session = &task.meta.session;
            if session.is_empty() {
                continue;
            }
            let new_name = format!("{}-{session}", i + 1);
            let current = sessions.values().find(|s| {
                s.name == *session
                    || s.name.ends_with(&format!("-{session}"))
            });
            if let Some(current) = current {
                if current.name != new_name {
                    let _ = Command::new("tmux")
                        .args([
                            "rename-session",
                            "-t",
                            &current.name,
                            &new_name,
                        ])
                        .stderr(Stdio::null())
                        .status();
                }
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
            .find(|s| {
                s.name == *session
                    || s.name.ends_with(&format!("-{session}"))
            })
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
        let _ = Command::new("gh")
            .args(["pr", "view", "--web", &pr.number.to_string()])
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
        TaskStatus::Attached => "·",
    }
}

fn status_color(status: TaskStatus) -> Color {
    match status {
        TaskStatus::Ready => PINE,
        TaskStatus::Working => FOAM,
        TaskStatus::Input => GOLD,
        TaskStatus::Idle | TaskStatus::Attached => MUTED,
    }
}

// Fixed column positions (0-indexed within inner content area).
// Status and dots use absolute positions so they never shift.
const COL_STATUS_END: usize = 53;
const COL_CI: usize = 56;
const COL_APPROVE: usize = 60;
const COL_CODEX: usize = 64;

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
    if has_prs && w > COL_CODEX {
        hdr[COL_CI] = 'c';
        hdr[COL_APPROVE] = 'a';
        hdr[COL_CODEX] = 'x';
    }
    {
        // Split: " orch" (bold) + rest-of-title (muted) + dots area
        let prefix: String = hdr[..COL_CI.min(w)].iter().collect();
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

        if has_prs && w > COL_CODEX {
            spans.push(Span::styled("c", Style::default().fg(MUTED)));
            let gap1: String =
                hdr[COL_CI + 1..COL_APPROVE].iter().collect();
            spans.push(Span::raw(gap1));
            spans.push(Span::styled("a", Style::default().fg(MUTED)));
            let gap2: String =
                hdr[COL_APPROVE + 1..COL_CODEX].iter().collect();
            spans.push(Span::raw(gap2));
            spans.push(Span::styled("x", Style::default().fg(MUTED)));
            if COL_CODEX + 1 < w {
                let tail: String =
                    hdr[COL_CODEX + 1..].iter().collect();
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
                    (COL_STATUS_END + 1).saturating_sub(label.len());

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
                let tail_len = w.saturating_sub(COL_STATUS_END + 1);

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
                let text = format!(
                    "#{} {}",
                    pr.number,
                    truncate(&pr.title, 42)
                );
                place(&mut buf, 0, gutter);
                place(&mut buf, 4, &text);

                let ci_ch = match pr.ci_pass {
                    Some(true) => '·',
                    Some(false) => '✗',
                    None => '○',
                };
                let ap_ch = if pr.approved { '·' } else { '○' };
                let cx_ch = if pr.codex_reviewed { '·' } else { '·' };

                if w > COL_CI { buf[COL_CI] = ci_ch; }
                if w > COL_APPROVE { buf[COL_APPROVE] = ap_ch; }
                if w > COL_CODEX { buf[COL_CODEX] = cx_ch; }

                let prefix: String =
                    buf[..COL_CI.min(w)].iter().collect();
                let mut spans = vec![Span::styled(
                    prefix,
                    if sel {
                        Style::default().fg(TEXT)
                    } else {
                        Style::default().fg(SUBTLE)
                    },
                )];

                if w > COL_CI {
                    let ci_style = match pr.ci_pass {
                        Some(true) => Style::default().fg(PINE),
                        Some(false) => Style::default().fg(LOVE),
                        None => Style::default().fg(MUTED),
                    };
                    spans.push(Span::styled(
                        ci_ch.to_string(), ci_style,
                    ));
                    let gap: String =
                        buf[COL_CI + 1..COL_APPROVE.min(w)]
                            .iter().collect();
                    spans.push(Span::raw(gap));
                }
                if w > COL_APPROVE {
                    let ap_style = if pr.approved {
                        Style::default().fg(PINE)
                    } else {
                        Style::default().fg(MUTED)
                    };
                    spans.push(Span::styled(
                        ap_ch.to_string(), ap_style,
                    ));
                    let gap: String =
                        buf[COL_APPROVE + 1..COL_CODEX.min(w)]
                            .iter().collect();
                    spans.push(Span::raw(gap));
                }
                if w > COL_CODEX {
                    let cx_style = if pr.codex_reviewed {
                        Style::default().fg(IRIS)
                    } else {
                        Style::default().fg(MUTED)
                    };
                    spans.push(Span::styled(
                        cx_ch.to_string(), cx_style,
                    ));
                    if COL_CODEX + 1 < w {
                        let tail: String =
                            buf[COL_CODEX + 1..].iter().collect();
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

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
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
        }
        if app.last_slow.elapsed() >= SLOW_TICK {
            app.refresh_slow();
            app.refresh_fast();
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
    match (key.modifiers, key.code) {
        (_, KeyCode::Char('q')) => Action::Quit,
        (KeyModifiers::CONTROL, KeyCode::Char('c')) => Action::Quit,
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
            app.reorder_down();
            Action::Continue
        }
        (_, KeyCode::Char('K')) => {
            app.reorder_up();
            Action::Continue
        }
        (_, KeyCode::Enter) => {
            match app.visible.get(app.cursor) {
                Some(ListItem::Task(_)) => {
                    // Toggle expand if has PRs, else jump to tmux
                    if app.selected_task().is_some_and(|t| !t.prs.is_empty()) {
                        app.toggle_expand();
                        Action::Continue
                    } else {
                        Action::Suspend(Box::new(|a| a.jump_to_session()))
                    }
                }
                Some(ListItem::Pr(_, _)) => {
                    // Open PR in browser for now (diff pane later)
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
                        codex_reviewed: true,
                    },
                    state::PrData {
                        number: 25812,
                        title: "add lock_wait_timeout".into(),
                        ci_pass: Some(true),
                        approved: false,
                        codex_reviewed: true,
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

    fn render_to_string(
        tasks: Vec<Task>,
        cursor: usize,
        width: u16,
        height: u16,
    ) -> String {
        // Expand all tasks with PRs so tests see PR rows
        let mut expanded = HashSet::new();
        for (i, t) in tasks.iter().enumerate() {
            if !t.prs.is_empty() {
                expanded.insert(i);
            }
        }
        let visible =
            App::build_visible(&tasks, &expanded);
        let app = App {
            tasks,
            order: Vec::new(),
            expanded,
            visible,
            cursor,
            pr_cache: PrCache::new(),
            last_fast: Instant::now(),
            last_slow: Instant::now(),
            should_quit: false,
        };
        let mut terminal =
            Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|f| render(f, &app))
            .unwrap();
        format!("{}", terminal.backend())
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
                    codex_reviewed: true,
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

        // Codex: 'x' vs rightmost ·
        let cx_hdr =
            rfind_char_col(header_line, 'x').unwrap();
        let cx_dot =
            rfind_char_col(pr_line, '·').unwrap();
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
}
