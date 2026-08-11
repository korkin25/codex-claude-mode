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
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::text::Text;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Wrap;

use crate::model::AgentThread;
use crate::prompt::PromptResolution;
use crate::prompt::ServerPrompt;

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
    ResolvePrompt(PromptResolution),
    Interrupt,
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
    pub(crate) prompt: Option<ServerPrompt>,
    input_history: Vec<String>,
    history_cursor: Option<usize>,
    history_draft: String,
    agent_window_start: usize,
    agent_hitboxes: Vec<(Rect, usize)>,
    agents_area: Rect,
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
            prompt: None,
            input_history: Vec::new(),
            history_cursor: None,
            history_draft: String::new(),
            agent_window_start: 0,
            agent_hitboxes: Vec::new(),
            agents_area: Rect::default(),
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
        let mut order = vec![root.clone()];
        append_children(&mut order, &root, &self.threads);
        self.order = order;
        self.root_id = Some(root);
        self.selected = selected_id
            .and_then(|id| self.order.iter().position(|candidate| candidate == &id))
            .unwrap_or(0);
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> Action {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('q') {
            return Action::Quit;
        }
        if let Some(prompt) = self.prompt.as_mut() {
            let resolution = prompt.handle_key(key, &mut self.input);
            if let Some(resolution) = resolution {
                self.prompt = None;
                self.input.clear();
                return Action::ResolvePrompt(resolution);
            }
            return Action::None;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Action::Interrupt;
        }
        match self.mode {
            Mode::Editing => match key.code {
                KeyCode::Esc => self.mode = Mode::Navigation,
                KeyCode::Enter if !self.input.trim().is_empty() => {
                    let input = std::mem::take(&mut self.input);
                    if self.input_history.last() != Some(&input) {
                        self.input_history.push(input.clone());
                    }
                    self.history_cursor = None;
                    self.history_draft.clear();
                    return Action::Submit(input);
                }
                KeyCode::Backspace => {
                    self.input.pop();
                    self.history_cursor = None;
                }
                KeyCode::Up => self.older_input(),
                KeyCode::Down => self.newer_input(),
                KeyCode::Char(character) => {
                    self.input.push(character);
                    self.history_cursor = None;
                }
                _ => {}
            },
            Mode::Navigation => match key.code {
                KeyCode::Enter => self.mode = Mode::Editing,
                KeyCode::Char(character)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    self.mode = Mode::Editing;
                    self.input.push(character);
                    self.history_cursor = None;
                }
                KeyCode::Left | KeyCode::Up => {
                    self.selected = self.selected.saturating_sub(1);
                    self.scroll = u16::MAX;
                    return Action::SelectionChanged;
                }
                KeyCode::Right | KeyCode::Down => {
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
        if self.prompt.is_some() {
            return Action::None;
        }
        match event.kind {
            MouseEventKind::Down(MouseButton::Left)
                if self
                    .agents_area
                    .contains(ratatui::layout::Position::new(event.column, event.row)) =>
            {
                if let Some((_, index)) = self.agent_hitboxes.iter().find(|(area, _)| {
                    area.contains(ratatui::layout::Position::new(event.column, event.row))
                }) {
                    self.selected = *index;
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
            .constraints([
                Constraint::Min(5),
                Constraint::Length(3),
                Constraint::Length(3),
            ])
            .split(frame.area());
        self.composer_area = outer[1];
        let footer = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(68), Constraint::Percentage(32)])
            .split(outer[2]);
        self.agents_area = footer[0];

        let selected = self.selected_thread();
        let log = self.prompt.as_ref().map_or_else(
            || {
                selected
                    .map(|thread| thread.log.join("\n\n"))
                    .filter(|log| !log.is_empty())
                    .unwrap_or_else(|| self.status_line.clone())
            },
            ServerPrompt::body,
        );
        let log_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(outer[0]);
        let header = self.prompt.as_ref().map_or_else(
            || {
                selected.map_or_else(
                    || self.status_line.clone(),
                    |thread| format!("{}  ·  {}", thread.label, thread.status),
                )
            },
            |_| "Action required".to_string(),
        );
        frame.render_widget(Paragraph::new(header.cyan().bold()), log_layout[0]);
        self.log_area = log_layout[1];
        let visible_height = self.log_area.height;
        let line_count = textwrap::wrap(&log, self.log_area.width.max(1) as usize)
            .len()
            .min(u16::MAX as usize) as u16;
        let max_scroll = line_count.saturating_sub(visible_height);
        let scroll = self.scroll.min(max_scroll);
        frame.render_widget(
            Paragraph::new(Text::raw(log))
                .wrap(Wrap { trim: false })
                .scroll((scroll, 0)),
            self.log_area,
        );

        let title = self.prompt.as_ref().map_or_else(
            || match self.mode {
                Mode::Editing => " Message · ↑/↓ history · Esc agents ",
                Mode::Navigation => " Message · Enter edit ",
            },
            ServerPrompt::composer_title,
        );
        let displayed_input = if self.prompt.as_ref().is_some_and(ServerPrompt::masks_input) {
            "•".repeat(self.input.chars().count())
        } else {
            self.input.clone()
        };
        frame.render_widget(
            Paragraph::new(displayed_input).block(
                Block::new()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray))
                    .title(title),
            ),
            self.composer_area,
        );

        self.render_agent_bar(frame);
        self.render_metrics(frame, footer[1]);
        let show_cursor = self
            .prompt
            .as_ref()
            .map_or(self.mode == Mode::Editing, ServerPrompt::accepts_text);
        if show_cursor {
            let cursor_x = self
                .composer_area
                .x
                .saturating_add(1)
                .saturating_add(self.input.chars().count() as u16)
                .min(self.composer_area.right().saturating_sub(2));
            frame.set_cursor_position((cursor_x, self.composer_area.y + 1));
        }
    }

    fn render_agent_bar(&mut self, frame: &mut Frame) {
        let entries = self
            .order
            .iter()
            .filter_map(|id| {
                self.threads.get(id).map(|thread| {
                    format!(
                        " {} {} {} ",
                        thread.label,
                        status_marker(&thread.status),
                        short_id(&thread.id)
                    )
                })
            })
            .collect::<Vec<_>>();
        let available = self.agents_area.width.saturating_sub(2) as usize;
        if self.selected < self.agent_window_start {
            self.agent_window_start = self.selected;
        }
        while self.agent_window_start < self.selected
            && entries[self.agent_window_start..=self.selected]
                .iter()
                .map(|entry| entry.chars().count() + 1)
                .sum::<usize>()
                > available
        {
            self.agent_window_start += 1;
        }

        self.agent_hitboxes.clear();
        let mut spans = Vec::new();
        let mut used = 0;
        for (index, entry) in entries.iter().enumerate().skip(self.agent_window_start) {
            let width = entry.chars().count();
            if used + width > available {
                break;
            }
            let style = if index == self.selected {
                Style::default().bg(Color::Rgb(42, 50, 56))
            } else {
                agent_status_style(self.threads[&self.order[index]].status.as_str())
            };
            spans.push(Span::styled(entry.clone(), style));
            self.agent_hitboxes.push((
                Rect::new(
                    self.agents_area.x + 1 + used as u16,
                    self.agents_area.y + 1,
                    width as u16,
                    1,
                ),
                index,
            ));
            used += width;
        }
        frame.render_widget(
            Paragraph::new(Line::from(spans)).block(
                Block::new()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray))
                    .title(" Agents · ←/→ select "),
            ),
            self.agents_area,
        );
    }

    fn render_metrics(&self, frame: &mut Frame, area: Rect) {
        let metrics = self.selected_thread().map_or_else(
            || self.status_line.clone(),
            |thread| {
                format!(
                    "{} · {} · in {} out {} total {} · {}",
                    thread.status,
                    format_duration(thread.elapsed()),
                    thread.tokens.input,
                    thread.tokens.output,
                    thread.tokens.total,
                    thread.id
                )
            },
        );
        frame.render_widget(
            Paragraph::new(metrics).block(
                Block::new()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray))
                    .title(" Activity "),
            ),
            area,
        );
    }

    pub(crate) fn set_prompt(&mut self, prompt: ServerPrompt) -> Result<(), Box<ServerPrompt>> {
        if self.prompt.is_some() {
            return Err(Box::new(prompt));
        }
        self.input.clear();
        self.prompt = Some(prompt);
        Ok(())
    }

    fn older_input(&mut self) {
        if self.input_history.is_empty() {
            return;
        }
        let index = match self.history_cursor {
            Some(index) => index.saturating_sub(1),
            None => {
                self.history_draft.clone_from(&self.input);
                self.input_history.len() - 1
            }
        };
        self.history_cursor = Some(index);
        self.input.clone_from(&self.input_history[index]);
    }

    fn newer_input(&mut self) {
        let Some(index) = self.history_cursor else {
            return;
        };
        if index + 1 < self.input_history.len() {
            self.history_cursor = Some(index + 1);
            self.input.clone_from(&self.input_history[index + 1]);
        } else {
            self.history_cursor = None;
            self.input.clone_from(&self.history_draft);
        }
    }

    pub(crate) fn clear_prompt(&mut self, request_id: &serde_json::Value) {
        if self
            .prompt
            .as_ref()
            .is_some_and(|prompt| &prompt.request_id == request_id)
        {
            self.prompt = None;
            self.input.clear();
        }
    }
}

fn append_children(order: &mut Vec<String>, parent: &str, threads: &HashMap<String, AgentThread>) {
    let mut children = threads
        .values()
        .filter(|thread| thread.parent_id.as_deref() == Some(parent))
        .collect::<Vec<_>>();
    children.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    for child in children {
        order.push(child.id.clone());
        append_children(order, &child.id, threads);
    }
}

fn short_id(id: &str) -> String {
    let characters = id.chars().collect::<Vec<_>>();
    if characters.len() <= 10 {
        return id.to_string();
    }
    format!(
        "{}…{}",
        characters[..4].iter().collect::<String>(),
        characters[characters.len() - 4..]
            .iter()
            .collect::<String>()
    )
}

fn agent_status_style(status: &str) -> Style {
    match status {
        "working" => Style::default().fg(Color::Green),
        "error" => Style::default().fg(Color::Red),
        "closed" => Style::default().fg(Color::DarkGray),
        _ => Style::default().fg(Color::Yellow),
    }
}

fn status_marker(status: &str) -> &'static str {
    match status {
        "working" => "●",
        "closed" => "○",
        "error" => "!",
        _ => "•",
    }
}

fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

#[cfg(test)]
#[path = "ui_tests.rs"]
mod tests;
