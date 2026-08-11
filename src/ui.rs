use std::collections::HashMap;
use std::time::Duration;

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use crossterm::event::MouseButton;
use crossterm::event::MouseEvent;
use crossterm::event::MouseEventKind;
use ratatui::Frame;
use ratatui::layout::Constraint;
use ratatui::layout::Direction;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Text;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Wrap;

use crate::model::AgentThread;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Navigation,
    Editing,
}

pub(crate) enum Action {
    None,
    Quit,
    Submit(String),
    SelectionChanged,
}

pub(crate) struct Workspace {
    pub(crate) threads: HashMap<String, AgentThread>,
    pub(crate) order: Vec<String>,
    pub(crate) selected: usize,
    pub(crate) root_id: Option<String>,
    pub(crate) mode: Mode,
    pub(crate) input: String,
    pub(crate) scroll: u16,
    pub(crate) status_line: String,
    tree_area: Rect,
    log_area: Rect,
    composer_area: Rect,
}

impl Workspace {
    pub(crate) fn new() -> Self {
        Self {
            threads: HashMap::new(),
            order: Vec::new(),
            selected: 0,
            root_id: None,
            mode: Mode::Editing,
            input: String::new(),
            scroll: u16::MAX,
            status_line: "connecting to installed Codex…".to_string(),
            tree_area: Rect::default(),
            log_area: Rect::default(),
            composer_area: Rect::default(),
        }
    }

    pub(crate) fn selected_id(&self) -> Option<&str> {
        self.order.get(self.selected).map(String::as_str)
    }

    pub(crate) fn selected_thread(&self) -> Option<&AgentThread> {
        self.selected_id().and_then(|id| self.threads.get(id))
    }

    pub(crate) fn rebuild_tree(&mut self, preferred_root: Option<&str>) {
        let root = preferred_root
            .and_then(|id| self.threads.get(id))
            .filter(|thread| thread.parent_id.is_none())
            .map(|thread| thread.id.clone())
            .or_else(|| {
                self.threads
                    .values()
                    .filter(|thread| thread.parent_id.is_none())
                    .max_by_key(|thread| thread.updated_at)
                    .map(|thread| thread.id.clone())
            });
        let Some(root) = root else {
            self.order.clear();
            self.root_id = None;
            self.selected = 0;
            return;
        };
        let selected_id = self.selected_id().map(ToOwned::to_owned);
        let session_id = self.threads[&root].session_id.clone();
        let mut order = vec![root.clone()];
        append_children(&mut order, &root, &session_id, &self.threads);
        self.order = order;
        self.root_id = Some(root);
        self.selected = selected_id
            .and_then(|id| self.order.iter().position(|candidate| candidate == &id))
            .unwrap_or(0);
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> Action {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Action::Quit;
        }
        match self.mode {
            Mode::Editing => match key.code {
                KeyCode::Esc => self.mode = Mode::Navigation,
                KeyCode::Enter if !self.input.trim().is_empty() => {
                    let input = std::mem::take(&mut self.input);
                    return Action::Submit(input);
                }
                KeyCode::Backspace => {
                    self.input.pop();
                }
                KeyCode::Char(character) => self.input.push(character),
                _ => {}
            },
            Mode::Navigation => match key.code {
                KeyCode::Char('q') => return Action::Quit,
                KeyCode::Enter | KeyCode::Char('i') => self.mode = Mode::Editing,
                KeyCode::Up => {
                    self.selected = self.selected.saturating_sub(1);
                    self.scroll = u16::MAX;
                    return Action::SelectionChanged;
                }
                KeyCode::Down => {
                    self.selected = (self.selected + 1).min(self.order.len().saturating_sub(1));
                    self.scroll = u16::MAX;
                    return Action::SelectionChanged;
                }
                KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(10),
                KeyCode::PageDown => self.scroll = self.scroll.saturating_add(10),
                KeyCode::Home => self.scroll = 0,
                KeyCode::End => self.scroll = u16::MAX,
                _ => {}
            },
        }
        Action::None
    }

    pub(crate) fn handle_mouse(&mut self, event: MouseEvent) -> Action {
        match event.kind {
            MouseEventKind::Down(MouseButton::Left)
                if self
                    .tree_area
                    .contains(ratatui::layout::Position::new(event.column, event.row)) =>
            {
                let row = event.row.saturating_sub(self.tree_area.y + 1) as usize;
                if row < self.order.len() {
                    self.selected = row;
                    self.mode = Mode::Navigation;
                    self.scroll = u16::MAX;
                    return Action::SelectionChanged;
                }
            }
            MouseEventKind::Down(MouseButton::Left)
                if self
                    .composer_area
                    .contains(ratatui::layout::Position::new(event.column, event.row)) =>
            {
                self.mode = Mode::Editing;
            }
            MouseEventKind::ScrollUp
                if self
                    .log_area
                    .contains(ratatui::layout::Position::new(event.column, event.row)) =>
            {
                self.mode = Mode::Navigation;
                self.scroll = self.scroll.saturating_sub(3);
            }
            MouseEventKind::ScrollDown
                if self
                    .log_area
                    .contains(ratatui::layout::Position::new(event.column, event.row)) =>
            {
                self.mode = Mode::Navigation;
                self.scroll = self.scroll.saturating_add(3);
            }
            _ => {}
        }
        Action::None
    }

    pub(crate) fn render(&mut self, frame: &mut Frame) {
        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(8), Constraint::Length(3)])
            .split(frame.area());
        let upper = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
            .split(outer[0]);
        self.tree_area = upper[0];
        self.composer_area = outer[1];

        let right = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(3)])
            .split(upper[1]);
        self.log_area = right[1];

        let tree = self
            .order
            .iter()
            .enumerate()
            .filter_map(|(index, id)| self.threads.get(id).map(|thread| (index, thread)))
            .map(|(index, thread)| {
                let depth = depth(thread, &self.threads);
                let marker = if index == self.selected { "›" } else { " " };
                let dot = match thread.status.as_str() {
                    "working" => "●".green(),
                    "error" => "●".red(),
                    "closed" => "○".dark_gray(),
                    _ => "●".yellow(),
                };
                Line::from(vec![
                    format!("{marker} {}", "  ".repeat(depth)).into(),
                    dot,
                    " ".into(),
                    thread.label.clone().into(),
                ])
            })
            .collect::<Vec<_>>();
        frame.render_widget(
            Paragraph::new(tree).block(Block::new().borders(Borders::ALL).title(" Agents ")),
            self.tree_area,
        );

        let selected = self.selected_thread();
        let metrics = selected.map_or_else(
            || self.status_line.clone(),
            |thread| {
                let elapsed = format_duration(thread.elapsed());
                format!(
                    " {} · {} · in {} / out {} / total {} · {} ",
                    thread.status,
                    elapsed,
                    thread.tokens.input,
                    thread.tokens.output,
                    thread.tokens.total,
                    thread.id
                )
            },
        );
        frame.render_widget(
            Paragraph::new(metrics).block(Block::new().borders(Borders::ALL).title(" Activity ")),
            right[0],
        );

        let log = selected
            .map(|thread| thread.log.join("\n\n"))
            .filter(|log| !log.is_empty())
            .unwrap_or_else(|| self.status_line.clone());
        let visible_height = self.log_area.height.saturating_sub(2);
        let line_count = textwrap::wrap(&log, self.log_area.width.saturating_sub(2) as usize)
            .len()
            .min(u16::MAX as usize) as u16;
        let max_scroll = line_count.saturating_sub(visible_height);
        let scroll = self.scroll.min(max_scroll);
        frame.render_widget(
            Paragraph::new(Text::raw(log))
                .wrap(Wrap { trim: false })
                .scroll((scroll, 0))
                .block(Block::new().borders(Borders::ALL).title(" Agent log ")),
            self.log_area,
        );

        let title = match self.mode {
            Mode::Editing => " Message · Esc navigation ",
            Mode::Navigation => " Navigation · Enter message · q quit ",
        };
        frame.render_widget(
            Paragraph::new(self.input.as_str())
                .block(Block::new().borders(Borders::ALL).title(title)),
            self.composer_area,
        );
        if self.mode == Mode::Editing {
            let cursor_x = self
                .composer_area
                .x
                .saturating_add(1)
                .saturating_add(self.input.chars().count() as u16)
                .min(self.composer_area.right().saturating_sub(2));
            frame.set_cursor_position((cursor_x, self.composer_area.y + 1));
        }
    }
}

fn append_children(
    order: &mut Vec<String>,
    parent: &str,
    session_id: &str,
    threads: &HashMap<String, AgentThread>,
) {
    let mut children = threads
        .values()
        .filter(|thread| {
            thread.session_id == session_id && thread.parent_id.as_deref() == Some(parent)
        })
        .collect::<Vec<_>>();
    children.sort_by_key(|thread| thread.updated_at);
    for child in children {
        order.push(child.id.clone());
        append_children(order, &child.id, session_id, threads);
    }
}

fn depth(thread: &AgentThread, threads: &HashMap<String, AgentThread>) -> usize {
    let mut depth = 0;
    let mut parent = thread.parent_id.as_deref();
    while let Some(parent_id) = parent {
        depth += 1;
        parent = threads
            .get(parent_id)
            .and_then(|thread| thread.parent_id.as_deref());
    }
    depth.min(4)
}

fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

#[cfg(test)]
#[path = "ui_tests.rs"]
mod tests;
