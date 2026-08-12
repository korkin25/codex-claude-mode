use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

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
use ratatui::widgets::Clear;
use ratatui::widgets::Paragraph;

use crate::command;
use crate::model::AgentThread;
use crate::model::LogEntry;
use crate::model::LogKind;
use crate::project_tree::{BrowserAction, EditorKind, ProjectBrowser};
use crate::prompt::PromptResolution;
use crate::prompt::ServerPrompt;
use crate::session::SessionCandidate;
use crate::shell_completion;
use crate::version;

const APP_BACKGROUND: Color = Color::Rgb(31, 34, 35);
const SURFACE_BACKGROUND: Color = Color::Rgb(47, 49, 50);
const SELECTED_BACKGROUND: Color = Color::Rgb(58, 61, 62);

fn count_line_breaks(text: &str) -> usize {
    let bytes = text.as_bytes();
    let mut count = 0;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\r' if bytes.get(index + 1) == Some(&b'\n') => {
                count += 1;
                index += 2;
            }
            b'\r' | b'\n' => {
                count += 1;
                index += 1;
            }
            _ => index += 1,
        }
    }
    count
}

fn push_text_input(inputs: &mut Vec<SubmissionInput>, text: String) {
    if text.is_empty() {
        return;
    }
    if let Some(SubmissionInput::Text(previous)) = inputs.last_mut() {
        previous.push_str(&text);
    } else {
        inputs.push(SubmissionInput::Text(text));
    }
}

fn human_size(size: usize) -> String {
    if size >= 1024 * 1024 {
        format!("{:.1} MB", size as f64 / (1024 * 1024) as f64)
    } else if size >= 1024 {
        format!("{:.1} KB", size as f64 / 1024.0)
    } else {
        format!("{size} B")
    }
}
const ACCENT_CYAN: Color = Color::Rgb(27, 181, 190);
const ACCENT_GREEN: Color = Color::Rgb(63, 185, 128);
const MUTED_TEXT: Color = Color::Rgb(132, 139, 141);
const BORDER_MUTED: Color = Color::Rgb(73, 77, 78);
const APPROVAL_BACKGROUND: Color = Color::Rgb(63, 52, 32);
const APPROVAL_BORDER: Color = Color::Rgb(224, 170, 70);

fn skill_query(text: &str, cursor: usize) -> Option<(usize, &str)> {
    let start = text[..cursor]
        .char_indices()
        .rev()
        .find(|(_, character)| character.is_whitespace())
        .map_or(0, |(index, character)| index + character.len_utf8());
    let token = &text[start..cursor];
    let query = token.strip_prefix('$')?;
    query
        .chars()
        .all(|character| character.is_alphanumeric() || matches!(character, '-' | '_'))
        .then_some((start, query))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Navigation,
    Editing,
}

pub(crate) enum Action {
    None,
    Quit,
    Submit(Submission),
    PasteImage,
    SelectionChanged,
    ResolvePrompt(PromptResolution),
    Interrupt,
    SessionSelected(Option<String>),
    ChooseSession,
    NewSession,
    UpdateCodex,
    OpenTerminalEditor {
        path: PathBuf,
        line: usize,
        column: usize,
    },
    OpenVsCode {
        path: PathBuf,
        line: usize,
        column: usize,
    },
    OpenCursor {
        path: PathBuf,
        line: usize,
        column: usize,
    },
    PermissionSelected {
        target_id: String,
        profile_id: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Submission {
    pub(crate) displayed_text: String,
    pub(crate) input: Vec<SubmissionInput>,
}

impl PartialEq<&str> for Submission {
    fn eq(&self, other: &&str) -> bool {
        matches!(self.input.as_slice(), [SubmissionInput::Text(text)] if text == other)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SubmissionInput {
    Text(String),
    LocalImage(PathBuf),
    Skill { name: String, path: PathBuf },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SkillChoice {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) path: PathBuf,
}

struct SessionPicker {
    candidates: Vec<SessionCandidate>,
    selected: usize,
    starting_new: bool,
}

pub(crate) struct PermissionChoice {
    pub(crate) id: String,
    pub(crate) description: String,
}

struct PermissionPicker {
    target_id: String,
    choices: Vec<PermissionChoice>,
    selected: usize,
}

struct CompletionPopup {
    start: usize,
    end: usize,
    candidates: Vec<String>,
    selected: usize,
}

struct SkillPopup {
    start: usize,
    end: usize,
    candidates: Vec<usize>,
    selected: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SkillBinding {
    start: usize,
    end: usize,
    name: String,
    path: PathBuf,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ComposerState {
    text: String,
    skill_bindings: Vec<SkillBinding>,
}

struct PastedText {
    placeholder: String,
    text: String,
}

struct PastedImage {
    placeholder: String,
    path: PathBuf,
}

pub(crate) struct Workspace {
    pub(crate) threads: HashMap<String, AgentThread>,
    pub(crate) order: Vec<String>,
    pub(crate) selected: usize,
    pub(crate) root_id: Option<String>,
    pub(crate) mode: Mode,
    pub(crate) input: String,
    input_cursor: Option<usize>,
    pub(crate) scroll: u16,
    last_max_scroll: u16,
    pub(crate) status_line: String,
    permission_profiles: HashMap<String, String>,
    completion_cwd: PathBuf,
    completion_popup: Option<CompletionPopup>,
    skills: Vec<SkillChoice>,
    skill_popup: Option<SkillPopup>,
    skill_bindings: Vec<SkillBinding>,
    backend_user_agent: Option<String>,
    codex_current_version: Option<String>,
    codex_latest_version: Option<String>,
    codex_update_confirm: bool,
    codex_update_running: bool,
    codex_update_result: Option<String>,
    pub(crate) prompt: Option<ServerPrompt>,
    prompt_draft: Option<String>,
    prompt_draft_cursor: Option<usize>,
    prompt_draft_skill_bindings: Vec<SkillBinding>,
    input_history: Vec<ComposerState>,
    history_cursor: Option<usize>,
    history_draft: ComposerState,
    pasted_texts: Vec<PastedText>,
    pasted_images: Vec<PastedImage>,
    next_paste_number: usize,
    agent_window_start: usize,
    agent_hitboxes: Vec<(Rect, usize)>,
    agents_area: Rect,
    log_area: Rect,
    composer_area: Rect,
    session_picker: Option<SessionPicker>,
    session_hitboxes: Vec<(Rect, usize)>,
    quit_armed: bool,
    info_open: bool,
    info_scroll: u16,
    slash_selected: usize,
    permission_picker: Option<PermissionPicker>,
    suspended_permission_picker: Option<PermissionPicker>,
    patch_open: bool,
    patch_scroll: u16,
    project_browser: Option<ProjectBrowser>,
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
            input_cursor: None,
            scroll: u16::MAX,
            last_max_scroll: 0,
            status_line: "connecting to installed Codex…".to_string(),
            permission_profiles: HashMap::new(),
            completion_cwd: std::env::current_dir().unwrap_or_default(),
            completion_popup: None,
            skills: Vec::new(),
            skill_popup: None,
            skill_bindings: Vec::new(),
            backend_user_agent: None,
            codex_current_version: None,
            codex_latest_version: None,
            codex_update_confirm: false,
            codex_update_running: false,
            codex_update_result: None,
            prompt: None,
            prompt_draft: None,
            prompt_draft_cursor: None,
            prompt_draft_skill_bindings: Vec::new(),
            input_history: Vec::new(),
            history_cursor: None,
            history_draft: ComposerState::default(),
            pasted_texts: Vec::new(),
            pasted_images: Vec::new(),
            next_paste_number: 1,
            agent_window_start: 0,
            agent_hitboxes: Vec::new(),
            agents_area: Rect::default(),
            log_area: Rect::default(),
            composer_area: Rect::default(),
            session_picker: None,
            session_hitboxes: Vec::new(),
            quit_armed: false,
            info_open: false,
            info_scroll: 0,
            slash_selected: 0,
            permission_picker: None,
            suspended_permission_picker: None,
            patch_open: false,
            patch_scroll: 0,
            project_browser: None,
        }
    }

    pub(crate) fn show_session_picker(&mut self, candidates: Vec<SessionCandidate>) {
        self.session_picker = Some(SessionPicker {
            candidates,
            selected: 0,
            starting_new: false,
        });
        self.mode = Mode::Navigation;
    }

    pub(crate) fn clear_session_picker(&mut self) {
        self.session_picker = None;
        self.session_hitboxes.clear();
    }

    pub(crate) fn show_permission_picker(
        &mut self,
        target_id: String,
        choices: Vec<PermissionChoice>,
        current: Option<&str>,
    ) {
        let selected = current
            .and_then(|current| choices.iter().position(|choice| choice.id == current))
            .unwrap_or(0);
        self.permission_picker = Some(PermissionPicker {
            target_id,
            choices,
            selected,
        });
        self.mode = Mode::Navigation;
    }

    pub(crate) fn set_permission_profile(&mut self, thread_id: &str, profile_id: &str) {
        self.permission_profiles
            .insert(thread_id.to_string(), profile_id.to_string());
    }

    pub(crate) fn set_completion_cwd(&mut self, cwd: PathBuf) {
        self.completion_cwd = cwd;
        self.completion_popup = None;
    }

    pub(crate) fn set_skills(&mut self, skills: Vec<SkillChoice>) {
        self.skills = skills;
        self.refresh_skill_popup();
    }

    pub(crate) fn set_backend_user_agent(&mut self, user_agent: &str) {
        self.backend_user_agent = (!user_agent.trim().is_empty()).then(|| user_agent.to_string());
    }

    fn scroll_log_up(&mut self, amount: u16) {
        self.scroll = self.scroll.min(self.last_max_scroll).saturating_sub(amount);
    }

    fn scroll_log_down(&mut self, amount: u16) {
        let next = self.scroll.min(self.last_max_scroll).saturating_add(amount);
        self.scroll = if next >= self.last_max_scroll {
            u16::MAX
        } else {
            next
        };
    }

    pub(crate) fn set_codex_versions(&mut self, current: Option<String>, latest: Option<String>) {
        self.codex_current_version = current;
        self.codex_latest_version = latest;
    }

    pub(crate) fn codex_update_started(&mut self) {
        self.codex_update_confirm = false;
        self.codex_update_running = true;
        self.codex_update_result = None;
    }

    pub(crate) fn codex_update_finished(&mut self, result: String) {
        self.codex_update_running = false;
        self.codex_update_result = Some(result);
    }

    pub(crate) fn show_session_starting(&mut self) {
        match self.session_picker.as_mut() {
            Some(picker) => picker.starting_new = true,
            None => {
                self.session_picker = Some(SessionPicker {
                    candidates: Vec::new(),
                    selected: 0,
                    starting_new: true,
                });
            }
        }
        self.session_hitboxes.clear();
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
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('d') {
            if self.quit_armed {
                return Action::Quit;
            }
            self.quit_armed = true;
            self.mode = Mode::Navigation;
            self.status_line = "Ctrl-D again to quit".to_string();
            return Action::None;
        }
        self.quit_armed = false;
        if let Some(picker) = self.session_picker.as_mut() {
            if picker.starting_new {
                return Action::None;
            }
            match key.code {
                KeyCode::Up | KeyCode::Left => {
                    picker.selected = picker.selected.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Right => {
                    picker.selected = (picker.selected + 1).min(picker.candidates.len());
                }
                KeyCode::Enter => {
                    let selected = picker.selected.checked_sub(1).and_then(|index| {
                        picker
                            .candidates
                            .get(index)
                            .map(|candidate| candidate.id.clone())
                    });
                    return Action::SessionSelected(selected);
                }
                _ => {}
            }
            return Action::None;
        }
        if let Some(picker) = self.permission_picker.as_mut() {
            match key.code {
                KeyCode::Up | KeyCode::Left => {
                    picker.selected = picker.selected.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Right => {
                    picker.selected =
                        (picker.selected + 1).min(picker.choices.len().saturating_sub(1));
                }
                KeyCode::Enter => {
                    let target_id = picker.target_id.clone();
                    let profile_id = picker.choices[picker.selected].id.clone();
                    self.permission_picker = None;
                    return Action::PermissionSelected {
                        target_id,
                        profile_id,
                    };
                }
                KeyCode::Esc => self.permission_picker = None,
                _ => {}
            }
            return Action::None;
        }
        if self.skill_popup.is_some() {
            match key.code {
                KeyCode::Up => {
                    let popup = self.skill_popup.as_mut().expect("skill popup");
                    popup.selected = popup.selected.saturating_sub(1);
                    return Action::None;
                }
                KeyCode::Down => {
                    let popup = self.skill_popup.as_mut().expect("skill popup");
                    popup.selected =
                        (popup.selected + 1).min(popup.candidates.len().saturating_sub(1));
                    return Action::None;
                }
                KeyCode::Enter | KeyCode::Tab => {
                    self.apply_selected_skill();
                    return Action::None;
                }
                KeyCode::Esc => {
                    self.skill_popup = None;
                    return Action::None;
                }
                _ => self.skill_popup = None,
            }
        }
        if self.completion_popup.is_some() && command::matches(&self.input).is_empty() {
            match key.code {
                KeyCode::Up => {
                    let popup = self.completion_popup.as_mut().expect("completion popup");
                    popup.selected = popup.selected.saturating_sub(1);
                    return Action::None;
                }
                KeyCode::Down => {
                    let popup = self.completion_popup.as_mut().expect("completion popup");
                    popup.selected =
                        (popup.selected + 1).min(popup.candidates.len().saturating_sub(1));
                    return Action::None;
                }
                KeyCode::Enter | KeyCode::Tab => {
                    self.apply_selected_completion();
                    return Action::None;
                }
                KeyCode::Esc => {
                    self.completion_popup = None;
                    return Action::None;
                }
                _ => self.completion_popup = None,
            }
        } else if self.completion_popup.is_some() {
            self.completion_popup = None;
        }
        if !self.patch_open
            && key.modifiers.contains(KeyModifiers::CONTROL)
            && key.code == KeyCode::Char('u')
        {
            let accepts_text = self.prompt.as_ref().is_some_and(ServerPrompt::accepts_text);
            if accepts_text || (self.prompt.is_none() && self.mode == Mode::Editing) {
                self.input.clear();
                self.skill_bindings.clear();
                self.input_cursor = None;
                self.history_cursor = None;
                self.history_draft = ComposerState::default();
                self.slash_selected = 0;
            }
            return Action::None;
        }
        if self.patch_open {
            match key.code {
                KeyCode::Char('q') => {
                    self.patch_open = false;
                }
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.patch_open = false;
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.patch_scroll = self.patch_scroll.saturating_sub(5)
                }
                KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.patch_scroll = self.patch_scroll.saturating_add(5)
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.patch_scroll = self.patch_scroll.saturating_sub(1)
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.patch_scroll = self.patch_scroll.saturating_add(1)
                }
                KeyCode::PageUp => self.patch_scroll = self.patch_scroll.saturating_sub(10),
                KeyCode::PageDown => self.patch_scroll = self.patch_scroll.saturating_add(10),
                KeyCode::Home => self.patch_scroll = 0,
                KeyCode::End => self.patch_scroll = u16::MAX,
                _ => {}
            }
            return Action::None;
        }
        if let Some(prompt) = self.prompt.as_mut() {
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && key.code == KeyCode::Char('a')
                && prompt.patch_text().is_some()
            {
                self.patch_open = true;
                self.patch_scroll = 0;
                return Action::None;
            }
            match key.code {
                KeyCode::PageUp => {
                    self.scroll_log_up(10);
                    return Action::None;
                }
                KeyCode::PageDown => {
                    self.scroll_log_down(10);
                    return Action::None;
                }
                KeyCode::Home => {
                    self.scroll = 0;
                    return Action::None;
                }
                KeyCode::End => {
                    self.scroll = u16::MAX;
                    return Action::None;
                }
                _ => {}
            }
            let resolution = prompt.handle_key(key, &mut self.input);
            if let Some(resolution) = resolution {
                self.finish_prompt();
                return Action::ResolvePrompt(resolution);
            }
            return Action::None;
        }
        if self.info_open {
            if self.codex_update_confirm {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                        return Action::UpdateCodex;
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                        self.codex_update_confirm = false;
                    }
                    _ => {}
                }
                return Action::None;
            }
            match key.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('i') => self.info_open = false,
                KeyCode::Char('u') | KeyCode::Char('U') if self.codex_update_available() => {
                    self.codex_update_confirm = true;
                }
                KeyCode::Up => self.info_scroll = self.info_scroll.saturating_sub(1),
                KeyCode::Down => self.info_scroll = self.info_scroll.saturating_add(1),
                KeyCode::PageUp => self.info_scroll = self.info_scroll.saturating_sub(8),
                KeyCode::PageDown => self.info_scroll = self.info_scroll.saturating_add(8),
                KeyCode::Home => self.info_scroll = 0,
                KeyCode::End => self.info_scroll = u16::MAX,
                _ => {}
            }
            return Action::None;
        }
        if let Some(browser) = self.project_browser.as_mut() {
            let action = browser.handle_key(key);
            return match action {
                BrowserAction::None => Action::None,
                BrowserAction::Close => {
                    self.project_browser = None;
                    Action::None
                }
                BrowserAction::OpenEditor { editor, path } => match editor {
                    EditorKind::Terminal => Action::OpenTerminalEditor {
                        path,
                        line: 1,
                        column: 1,
                    },
                    EditorKind::VsCode => Action::OpenVsCode {
                        path,
                        line: 1,
                        column: 1,
                    },
                    EditorKind::Cursor => Action::OpenCursor {
                        path,
                        line: 1,
                        column: 1,
                    },
                },
            };
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('a') => return self.prepare_subagent_request(),
                KeyCode::Char('n') => return Action::NewSession,
                KeyCode::Char('r') => return Action::ChooseSession,
                _ => {}
            }
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Action::Interrupt;
        }
        match self.mode {
            Mode::Editing => {
                let slash_matches = command::matches(&self.input);
                if !slash_matches.is_empty() {
                    match key.code {
                        KeyCode::Up => {
                            self.slash_selected = self.slash_selected.saturating_sub(1);
                            return Action::None;
                        }
                        KeyCode::Down => {
                            self.slash_selected =
                                (self.slash_selected + 1).min(slash_matches.len() - 1);
                            return Action::None;
                        }
                        KeyCode::Enter | KeyCode::Tab => {
                            let selected = self.slash_selected.min(slash_matches.len() - 1);
                            self.input = format!("/{} ", slash_matches[selected].name);
                            self.skill_bindings.clear();
                            self.input_cursor = None;
                            self.slash_selected = 0;
                            return Action::None;
                        }
                        _ => {}
                    }
                }
                match key.code {
                    KeyCode::Esc => self.mode = Mode::Navigation,
                    KeyCode::Char('i') if key.modifiers.contains(KeyModifiers::ALT) => {
                        return Action::PasteImage;
                    }
                    KeyCode::Left => {
                        self.move_cursor_left();
                        self.refresh_skill_popup();
                    }
                    KeyCode::Right => {
                        self.move_cursor_right();
                        self.refresh_skill_popup();
                    }
                    KeyCode::Enter if !self.input.trim().is_empty() => {
                        let displayed_input = std::mem::take(&mut self.input);
                        self.input_cursor = None;
                        let history_entry = ComposerState {
                            text: displayed_input.clone(),
                            skill_bindings: self.skill_bindings.clone(),
                        };
                        if self.input_history.last() != Some(&history_entry) {
                            self.input_history.push(history_entry);
                        }
                        self.history_cursor = None;
                        self.history_draft = ComposerState::default();
                        self.mode = Mode::Navigation;
                        let submission = self.build_submission(displayed_input);
                        self.skill_bindings.clear();
                        return Action::Submit(submission);
                    }
                    KeyCode::Tab => {
                        self.start_completion();
                    }
                    KeyCode::Backspace => {
                        self.backspace_at_cursor();
                        self.history_cursor = None;
                        self.slash_selected = 0;
                        self.refresh_skill_popup();
                    }
                    KeyCode::Up => self.older_input(),
                    KeyCode::Down => self.newer_input(),
                    KeyCode::PageUp => self.scroll_log_up(10),
                    KeyCode::PageDown => self.scroll_log_down(10),
                    KeyCode::Home => self.input_cursor = Some(0),
                    KeyCode::End => self.input_cursor = None,
                    KeyCode::Char(character) => {
                        self.insert_at_cursor(&character.to_string());
                        self.history_cursor = None;
                        self.slash_selected = 0;
                        self.refresh_skill_popup();
                    }
                    _ => {}
                }
            }
            Mode::Navigation => match key.code {
                KeyCode::Enter => self.mode = Mode::Editing,
                KeyCode::Char('t') => {
                    let root = self
                        .selected_thread()
                        .and_then(|thread| {
                            (!thread.cwd.is_empty()).then(|| PathBuf::from(&thread.cwd))
                        })
                        .unwrap_or_else(|| self.completion_cwd.clone());
                    self.project_browser = Some(ProjectBrowser::open(root));
                }
                KeyCode::Char('i') => {
                    self.info_open = true;
                    self.info_scroll = 0;
                }
                KeyCode::Char(character)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    self.mode = Mode::Editing;
                    self.insert_at_cursor(&character.to_string());
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
                KeyCode::PageUp => self.scroll_log_up(10),
                KeyCode::PageDown => self.scroll_log_down(10),
                KeyCode::Home => self.scroll = 0,
                KeyCode::End => self.scroll = u16::MAX,
                _ => {}
            },
        }
        Action::None
    }

    pub(crate) fn handle_paste(&mut self, text: String) -> Action {
        if self.session_picker.is_some() || self.info_open || self.permission_picker.is_some() {
            return Action::None;
        }
        if let Some(prompt) = self.prompt.as_ref() {
            if prompt.accepts_text() {
                self.input.push_str(&text);
                self.input_cursor = None;
            }
            return Action::None;
        }
        if self.mode == Mode::Navigation {
            self.mode = Mode::Editing;
        }
        if text.contains(['\n', '\r']) {
            let additional_lines = count_line_breaks(&text);
            let placeholder = format!(
                "[Pasted text #{} +{} lines]",
                self.next_paste_number, additional_lines
            );
            self.next_paste_number += 1;
            self.insert_at_cursor(&placeholder);
            self.pasted_texts.push(PastedText { placeholder, text });
        } else {
            self.insert_at_cursor(&text);
        }
        self.history_cursor = None;
        self.slash_selected = 0;
        self.completion_popup = None;
        Action::None
    }

    pub(crate) fn attach_image(&mut self, path: PathBuf, format: &str, size: usize) {
        let placeholder = format!(
            "[Image #{} {format} {}]",
            self.pasted_images.len() + 1,
            human_size(size)
        );
        self.insert_at_cursor(&placeholder);
        self.pasted_images.push(PastedImage { placeholder, path });
        self.mode = Mode::Editing;
        self.history_cursor = None;
        self.completion_popup = None;
        self.skill_popup = None;
        self.refresh_skill_popup();
    }

    pub(crate) fn handle_mouse(&mut self, event: MouseEvent) -> Action {
        if self.session_picker.is_some()
            && event.kind == MouseEventKind::Down(MouseButton::Left)
            && let Some((_, index)) = self.session_hitboxes.iter().find(|(area, _)| {
                area.contains(ratatui::layout::Position::new(event.column, event.row))
            })
        {
            let index = *index;
            if let Some(picker) = self.session_picker.as_mut() {
                picker.selected = index;
                let selected = index.checked_sub(1).and_then(|candidate_index| {
                    picker
                        .candidates
                        .get(candidate_index)
                        .map(|candidate| candidate.id.clone())
                });
                return Action::SessionSelected(selected);
            }
        }
        if self.session_picker.is_some() {
            return Action::None;
        }
        if self.permission_picker.is_some() {
            return Action::None;
        }
        if self.patch_open {
            return Action::None;
        }
        if self.info_open {
            return Action::None;
        }
        if self.prompt.is_some() {
            match event.kind {
                MouseEventKind::ScrollUp
                    if self
                        .log_area
                        .contains(ratatui::layout::Position::new(event.column, event.row)) =>
                {
                    self.scroll_log_up(3);
                }
                MouseEventKind::ScrollDown
                    if self
                        .log_area
                        .contains(ratatui::layout::Position::new(event.column, event.row)) =>
                {
                    self.scroll_log_down(3);
                }
                _ => {}
            }
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
                self.scroll_log_up(3);
            }
            MouseEventKind::ScrollDown
                if self
                    .log_area
                    .contains(ratatui::layout::Position::new(event.column, event.row)) =>
            {
                self.scroll_log_down(3);
            }
            _ => {}
        }
        Action::None
    }

    pub(crate) fn render(&mut self, frame: &mut Frame) {
        frame.render_widget(
            Block::new().style(Style::default().bg(APP_BACKGROUND)),
            frame.area(),
        );
        if self.session_picker.is_some() {
            self.render_session_picker(frame);
            return;
        }
        if self.prompt.is_none()
            && let Some(browser) = self.project_browser.as_mut()
        {
            browser.render(frame);
            return;
        }
        if self.patch_open {
            self.render_patch_view(frame);
            return;
        }
        let composer_height =
            if let Some(lines) = self.prompt.as_ref().and_then(ServerPrompt::decision_lines) {
                let width = frame.area().width.saturating_sub(2).max(1) as usize;
                let line_count = lines
                    .iter()
                    .map(|line| textwrap::wrap(&line.text, width).len().max(1))
                    .sum::<usize>();
                (line_count.saturating_add(2).min(u16::MAX as usize) as u16)
                    .min(frame.area().height.saturating_sub(5))
            } else {
                3
            };
        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(5),
                Constraint::Length(composer_height),
                Constraint::Length(3),
            ])
            .split(frame.area());
        self.composer_area = outer[1];
        let footer = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(68), Constraint::Percentage(32)])
            .split(outer[2]);
        self.agents_area = footer[0];

        let log_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(outer[0]);
        self.log_area = log_layout[1];
        let selected = self.selected_thread();
        let header = if self.quit_armed {
            "Ctrl-D again to quit · any other key cancels".to_string()
        } else {
            self.prompt.as_ref().map_or_else(
                || {
                    selected.map_or_else(
                        || self.status_line.clone(),
                        |thread| format!("{}  ·  {}", thread.label, thread.display_status()),
                    )
                },
                |_| "Action required".to_string(),
            )
        };
        frame.render_widget(Paragraph::new(header.fg(ACCENT_CYAN).bold()), log_layout[0]);
        let log_lines = self.prompt.as_ref().map_or_else(
            || {
                selected.map_or_else(
                    || plain_lines(&self.status_line, self.log_area.width),
                    |thread| {
                        if thread.log.is_empty() {
                            plain_lines(&self.status_line, self.log_area.width)
                        } else {
                            agent_log_lines(thread, self.log_area.width)
                        }
                    },
                )
            },
            |prompt| plain_lines(&prompt.body(), self.log_area.width),
        );
        let visible_height = self.log_area.height;
        let line_count = log_lines.len().min(u16::MAX as usize) as u16;
        let max_scroll = line_count.saturating_sub(visible_height);
        self.last_max_scroll = max_scroll;
        let scroll = self.scroll.min(max_scroll);
        frame.render_widget(
            Paragraph::new(Text::from(log_lines)).scroll((scroll, 0)),
            self.log_area,
        );

        let title = self.prompt.as_ref().map_or_else(
            || match self.mode {
                Mode::Editing => " Message · ↑/↓ history · Esc agents ",
                Mode::Navigation => " Message · Enter edit ",
            },
            ServerPrompt::composer_title,
        );
        let displayed_input = self.prompt.as_ref().map_or_else(
            || self.input.clone(),
            |prompt| {
                prompt.decision_text().unwrap_or_else(|| {
                    if prompt.masks_input() {
                        "•".repeat(self.input.chars().count())
                    } else {
                        self.input.clone()
                    }
                })
            },
        );
        let decision_prompt = self
            .prompt
            .as_ref()
            .is_some_and(|prompt| !prompt.accepts_text());
        let composer_border = if decision_prompt {
            APPROVAL_BORDER
        } else {
            match self.mode {
                Mode::Editing => ACCENT_CYAN,
                Mode::Navigation => BORDER_MUTED,
            }
        };
        let composer_background = if decision_prompt {
            APPROVAL_BACKGROUND
        } else {
            APP_BACKGROUND
        };
        let displayed_cursor = if self.prompt.is_some() {
            displayed_input.len()
        } else {
            self.actual_input_cursor()
        };
        let (composer_scroll, cursor_column, cursor_row) = if decision_prompt {
            (0, 0, 0)
        } else {
            composer_viewport(
                &displayed_input,
                displayed_cursor,
                self.composer_area.width.saturating_sub(2),
                self.composer_area.height.saturating_sub(2),
            )
        };
        let composer_width = self.composer_area.width.saturating_sub(2).max(1) as usize;
        let wrapped_input = wrap_composer_input(&displayed_input, composer_width).join("\n");
        let composer_text = self
            .prompt
            .as_ref()
            .and_then(ServerPrompt::decision_lines)
            .map_or_else(
                || Text::from(wrapped_input),
                |decision_lines| {
                    Text::from(
                        decision_lines
                            .into_iter()
                            .flat_map(|decision| {
                                let style = if decision.selected {
                                    Style::default()
                                        .bg(SELECTED_BACKGROUND)
                                        .fg(APPROVAL_BORDER)
                                        .bold()
                                } else {
                                    Style::default().fg(APPROVAL_BORDER).bold()
                                };
                                textwrap::wrap(&decision.text, composer_width)
                                    .into_iter()
                                    .map(move |line| {
                                        Line::from(Span::styled(line.into_owned(), style))
                                    })
                                    .collect::<Vec<_>>()
                            })
                            .collect::<Vec<_>>(),
                    )
                },
            );
        frame.render_widget(
            Paragraph::new(composer_text)
                .scroll((composer_scroll, 0))
                .style(
                    Style::default()
                        .bg(composer_background)
                        .fg(if decision_prompt {
                            APPROVAL_BORDER
                        } else {
                            Color::Reset
                        })
                        .bold(),
                )
                .block(
                    Block::new()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(composer_border))
                        .style(Style::default().bg(composer_background))
                        .title(title),
                ),
            self.composer_area,
        );

        self.render_completion_menu(frame);
        self.render_slash_menu(frame);
        self.render_skill_menu(frame);

        self.render_agent_bar(frame);
        self.render_metrics(frame, footer[1]);
        if self.info_open {
            self.render_info_overlay(frame);
            return;
        }
        if self.permission_picker.is_some() {
            self.render_permission_picker(frame);
            return;
        }
        let show_cursor = self
            .prompt
            .as_ref()
            .map_or(self.mode == Mode::Editing, ServerPrompt::accepts_text);
        if show_cursor {
            let cursor_x = self
                .composer_area
                .x
                .saturating_add(1)
                .saturating_add(cursor_column)
                .min(self.composer_area.right().saturating_sub(2));
            let cursor_y = self
                .composer_area
                .y
                .saturating_add(1)
                .saturating_add(cursor_row)
                .min(self.composer_area.bottom().saturating_sub(2));
            frame.set_cursor_position((cursor_x, cursor_y));
        }
    }

    fn render_slash_menu(&self, frame: &mut Frame) {
        let matches = command::matches(&self.input);
        if self.mode != Mode::Editing || matches.is_empty() || self.prompt.is_some() {
            return;
        }
        let visible = matches.len().min(8);
        let height = (visible as u16).saturating_add(2);
        let area = Rect::new(
            self.composer_area.x,
            self.composer_area.y.saturating_sub(height),
            self.composer_area.width,
            height.min(self.composer_area.y),
        );
        if area.height < 2 {
            return;
        }
        let selected = self.slash_selected.min(matches.len() - 1);
        let start = selected.saturating_sub(visible.saturating_sub(1));
        let lines = matches
            .iter()
            .enumerate()
            .skip(start)
            .take(visible)
            .map(|(index, command)| {
                let style = if index == selected {
                    Style::default().bg(SELECTED_BACKGROUND).bold()
                } else {
                    Style::default().bg(APP_BACKGROUND)
                };
                styled_full_line(
                    format!(" /{:<14} {}", command.name, command.description),
                    area.width.saturating_sub(2),
                    style,
                )
            })
            .collect::<Vec<_>>();
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(Text::from(lines)).block(
                Block::new()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(ACCENT_CYAN))
                    .style(Style::default().bg(APP_BACKGROUND))
                    .title(" Commands ")
                    .title_bottom(" ↑/↓ select · Enter/Tab insert "),
            ),
            area,
        );
    }

    fn render_completion_menu(&self, frame: &mut Frame) {
        let Some(popup) = self.completion_popup.as_ref() else {
            return;
        };
        if self.mode != Mode::Editing || self.prompt.is_some() {
            return;
        }
        let visible = popup.candidates.len().min(8);
        let height = (visible as u16).saturating_add(2);
        let area = Rect::new(
            self.composer_area.x,
            self.composer_area.y.saturating_sub(height),
            self.composer_area.width,
            height.min(self.composer_area.y),
        );
        if area.height < 2 {
            return;
        }
        let selected = popup.selected.min(popup.candidates.len().saturating_sub(1));
        let start = selected.saturating_sub(visible.saturating_sub(1));
        let lines = popup
            .candidates
            .iter()
            .enumerate()
            .skip(start)
            .take(visible)
            .map(|(index, candidate)| {
                let style = if index == selected {
                    Style::default().bg(SELECTED_BACKGROUND).bold()
                } else {
                    Style::default().bg(APP_BACKGROUND)
                };
                styled_full_line(format!(" {candidate}"), area.width.saturating_sub(2), style)
            })
            .collect::<Vec<_>>();
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(Text::from(lines)).block(
                Block::new()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(ACCENT_CYAN))
                    .style(Style::default().bg(APP_BACKGROUND))
                    .title(" Completions ")
                    .title_bottom(" ↑/↓ select · Enter/Tab insert · Esc close "),
            ),
            area,
        );
    }

    fn render_skill_menu(&self, frame: &mut Frame) {
        let Some(popup) = self.skill_popup.as_ref() else {
            return;
        };
        if self.mode != Mode::Editing || self.prompt.is_some() {
            return;
        }
        let visible = popup.candidates.len().min(8);
        let height = (visible as u16).saturating_add(2);
        let area = Rect::new(
            self.composer_area.x,
            self.composer_area.y.saturating_sub(height),
            self.composer_area.width,
            height.min(self.composer_area.y),
        );
        if area.height < 2 {
            return;
        }
        let selected = popup.selected.min(popup.candidates.len().saturating_sub(1));
        let start = selected.saturating_sub(visible.saturating_sub(1));
        let lines = popup
            .candidates
            .iter()
            .enumerate()
            .skip(start)
            .take(visible)
            .map(|(index, skill_index)| {
                let skill = &self.skills[*skill_index];
                let style = if index == selected {
                    Style::default().bg(SELECTED_BACKGROUND).bold()
                } else {
                    Style::default().bg(APP_BACKGROUND)
                };
                styled_full_line(
                    format!(" ${:<20} {}", skill.name, skill.description),
                    area.width.saturating_sub(2),
                    style,
                )
            })
            .collect::<Vec<_>>();
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(Text::from(lines)).block(
                Block::new()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(ACCENT_CYAN))
                    .style(Style::default().bg(APP_BACKGROUND))
                    .title(" Skills ")
                    .title_bottom(" ↑/↓ select · Enter/Tab insert · Esc close "),
            ),
            area,
        );
    }

    fn render_info_overlay(&self, frame: &mut Frame) {
        let Some(thread) = self.selected_thread() else {
            return;
        };
        let area = centered_info_area(frame.area());
        let root_id = self.root_id.as_deref().unwrap_or("unavailable");
        let session_id = nonempty_or(&thread.session_id, "unavailable");
        let parent_id = thread.parent_id.as_deref().unwrap_or("none (Main)");
        let cwd = nonempty_or(&thread.cwd, "unavailable");
        let log_path = thread
            .path
            .as_deref()
            .unwrap_or("not persisted / unavailable");
        let backend = self.backend_user_agent.as_deref().unwrap_or("unavailable");
        let body = format!(
            "Session ID: {session_id}\nRoot thread ID: {root_id}\nAgent: {}\nAgent thread ID: {}\nParent thread ID: {parent_id}\nStatus: {}\nWorking directory: {cwd}\nRollout / log: {log_path}",
            thread.label,
            thread.id,
            thread.display_status(),
        );
        frame.render_widget(Clear, area);
        let inner_height = area.height.saturating_sub(2);
        let current = self
            .codex_current_version
            .as_deref()
            .unwrap_or("unavailable");
        let latest = self
            .codex_latest_version
            .as_deref()
            .unwrap_or("unavailable");
        let update_available = self.codex_update_available();
        let version_style = if update_available {
            Style::default().red().bold()
        } else {
            Style::default().fg(ACCENT_GREEN)
        };
        let mut lines = vec![
            Line::from(Span::styled(
                format!("Codex version: {current} (latest: {latest})"),
                version_style,
            )),
            format!("Codex backend: {backend}").into(),
        ];
        if update_available {
            lines.push("Update available · U update".red().bold().into());
        }
        if self.codex_update_running {
            lines.push("Updating Codex…".yellow().bold().into());
        }
        if let Some(result) = &self.codex_update_result {
            lines.extend(plain_lines(result, area.width.saturating_sub(2)));
        }
        lines.extend(plain_lines(&body, area.width.saturating_sub(2)));
        let max_scroll = (lines.len().min(u16::MAX as usize) as u16).saturating_sub(inner_height);
        frame.render_widget(
            Paragraph::new(Text::from(lines))
                .scroll((self.info_scroll.min(max_scroll), 0))
                .block(
                    Block::new()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(ACCENT_CYAN))
                        .style(Style::default().bg(APP_BACKGROUND))
                        .title(" Session / agent info ")
                        .title_bottom(if self.codex_update_confirm {
                            " Update Codex now? y / Enter confirm · n / Esc cancel "
                        } else if update_available {
                            " U update · ↑/↓ scroll · i / Esc / Enter close "
                        } else {
                            " ↑/↓ scroll · i / Esc / Enter close "
                        }),
                ),
            area,
        );
    }

    fn codex_update_available(&self) -> bool {
        self.codex_current_version
            .as_deref()
            .zip(self.codex_latest_version.as_deref())
            .is_some_and(|(current, latest)| version::update_available(current, latest))
            && !self.codex_update_running
    }

    fn render_permission_picker(&self, frame: &mut Frame) {
        let Some(picker) = self.permission_picker.as_ref() else {
            return;
        };
        let area = centered_session_area(frame.area(), picker.choices.len());
        frame.render_widget(Clear, area);
        let lines = picker
            .choices
            .iter()
            .enumerate()
            .map(|(index, choice)| {
                let style = if index == picker.selected {
                    Style::default().bg(SELECTED_BACKGROUND).bold()
                } else {
                    Style::default().bg(APP_BACKGROUND)
                };
                styled_full_line(
                    format!(" {} · {}", choice.id, choice.description),
                    area.width.saturating_sub(2),
                    style,
                )
            })
            .collect::<Vec<_>>();
        frame.render_widget(
            Paragraph::new(Text::from(lines)).block(
                Block::new()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(ACCENT_CYAN))
                    .style(Style::default().bg(APP_BACKGROUND))
                    .title(" Permissions ")
                    .title_bottom(" ↑/↓ select · Enter apply · Esc cancel "),
            ),
            area,
        );
    }

    fn render_patch_view(&self, frame: &mut Frame) {
        let Some(patch) = self.prompt.as_ref().and_then(ServerPrompt::patch_text) else {
            return;
        };
        let area = frame.area();
        let block = Block::new()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(ACCENT_CYAN))
            .style(Style::default().bg(APP_BACKGROUND))
            .title(" P A T C H ")
            .title_bottom(" ↑/↓ · PgUp/PgDn · Home/End · q/Ctrl-C close ");
        let inner = block.inner(area);
        let lines = patch_lines(patch, inner.width);
        let max_scroll = (lines.len().min(u16::MAX as usize) as u16).saturating_sub(inner.height);
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(Text::from(lines))
                .scroll((self.patch_scroll.min(max_scroll), 0))
                .block(block),
            area,
        );
    }

    fn render_session_picker(&mut self, frame: &mut Frame) {
        let Some(picker) = self.session_picker.as_ref() else {
            return;
        };
        let area = centered_session_area(frame.area(), picker.candidates.len());
        frame.render_widget(Clear, area);
        let block = Block::new()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(ACCENT_CYAN))
            .style(Style::default().bg(APP_BACKGROUND))
            .title(" Choose session ")
            .title_bottom(" ↑/↓ select · Enter open ");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        self.session_hitboxes.clear();

        if picker.starting_new {
            frame.render_widget(
                Paragraph::new("Creating a clean Main session…".fg(ACCENT_CYAN).bold()).centered(),
                Rect::new(inner.x, inner.y, inner.width, inner.height),
            );
            return;
        }
        let mut rows = vec![("+ New session".to_string(), 0)];
        rows.extend(
            picker
                .candidates
                .iter()
                .enumerate()
                .map(|(index, candidate)| {
                    let preview = if candidate.preview.is_empty() {
                        "(empty session)"
                    } else {
                        candidate.preview.as_str()
                    };
                    (
                        format!(
                            "Continue · {} · {} · {}",
                            preview,
                            relative_time(candidate.updated_at),
                            short_id(&candidate.id)
                        ),
                        index + 1,
                    )
                }),
        );

        let available_width = inner.width.saturating_sub(2) as usize;
        for (row, (label, index)) in rows.into_iter().enumerate() {
            if row >= inner.height as usize {
                break;
            }
            let label = truncate_line(&label, available_width);
            let style = if index == picker.selected {
                Style::default().bg(SELECTED_BACKGROUND).bold()
            } else {
                Style::default().bg(APP_BACKGROUND)
            };
            let row_area = Rect::new(
                inner.x.saturating_add(1),
                inner.y + row as u16,
                inner.width.saturating_sub(2),
                1,
            );
            frame.render_widget(
                Paragraph::new(styled_full_line(label, row_area.width, style)),
                row_area,
            );
            self.session_hitboxes.push((row_area, index));
        }
    }

    fn render_agent_bar(&mut self, frame: &mut Frame) {
        let entries = self
            .order
            .iter()
            .filter_map(|id| {
                self.threads
                    .get(id)
                    .map(|thread| format!(" {} {} ", thread.label, status_marker(&thread.status)))
            })
            .collect::<Vec<_>>();
        let available = self.agents_area.width as usize;
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
                Style::default().bg(SELECTED_BACKGROUND).bold()
            } else {
                agent_status_style(self.threads[&self.order[index]].status.as_str())
            };
            spans.push(Span::styled(entry.clone(), style));
            self.agent_hitboxes.push((
                Rect::new(
                    self.agents_area.x + used as u16,
                    self.agents_area.y + 1,
                    width as u16,
                    1,
                ),
                index,
            ));
            used += width;
        }
        frame.render_widget(
            Paragraph::new(
                Line::from("Agents · ←/→ · i info · ^A agent · ^N new · ^R sessions")
                    .fg(MUTED_TEXT),
            ),
            Rect::new(
                self.agents_area.x,
                self.agents_area.y,
                self.agents_area.width,
                1,
            ),
        );
        frame.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect::new(
                self.agents_area.x,
                self.agents_area.y + 1,
                self.agents_area.width,
                1,
            ),
        );
    }

    fn render_metrics(&self, frame: &mut Frame, area: Rect) {
        let metrics = self.selected_thread().map_or_else(
            || Line::from(self.status_line.clone()).fg(MUTED_TEXT),
            |thread| {
                let mut spans = Vec::new();
                if let Some(profile) = self.permission_profiles.get(&thread.id) {
                    spans.push("permissions ".fg(MUTED_TEXT));
                    let style = if is_full_access_profile(profile) {
                        Style::default().fg(Color::Red).bold()
                    } else {
                        Style::default().fg(MUTED_TEXT)
                    };
                    spans.push(Span::styled(profile.clone(), style));
                    spans.push(" · ".fg(MUTED_TEXT));
                }
                spans.push(
                    format!(
                        "{} · {} · in {} out {} total {} · {}",
                        thread.display_status(),
                        format_duration(thread.elapsed()),
                        thread.tokens.input,
                        thread.tokens.output,
                        thread.tokens.total,
                        thread.id
                    )
                    .fg(MUTED_TEXT),
                );
                Line::from(spans)
            },
        );
        frame.render_widget(
            Paragraph::new("Activity".fg(MUTED_TEXT)),
            Rect::new(area.x, area.y, area.width, 1),
        );
        frame.render_widget(
            Paragraph::new(metrics),
            Rect::new(area.x, area.y + 1, area.width, 1),
        );
    }

    pub(crate) fn set_prompt(&mut self, prompt: ServerPrompt) -> Result<(), Box<ServerPrompt>> {
        if self.prompt.is_some() {
            return Err(Box::new(prompt));
        }
        self.prompt_draft = Some(std::mem::take(&mut self.input));
        self.prompt_draft_cursor = self.input_cursor.take();
        self.prompt_draft_skill_bindings = std::mem::take(&mut self.skill_bindings);
        self.completion_popup = None;
        self.skill_popup = None;
        self.info_open = false;
        self.suspended_permission_picker = self.permission_picker.take();
        self.patch_open = false;
        self.slash_selected = 0;
        self.scroll = 0;
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
                self.history_draft = ComposerState {
                    text: self.input.clone(),
                    skill_bindings: self.skill_bindings.clone(),
                };
                self.input_history.len() - 1
            }
        };
        self.history_cursor = Some(index);
        self.input.clone_from(&self.input_history[index].text);
        self.skill_bindings
            .clone_from(&self.input_history[index].skill_bindings);
        self.input_cursor = None;
    }

    fn actual_input_cursor(&self) -> usize {
        self.input_cursor
            .unwrap_or(self.input.len())
            .min(self.input.len())
    }

    fn insert_at_cursor(&mut self, text: &str) {
        let cursor = self.actual_input_cursor();
        self.update_skill_bindings_for_edit(cursor, cursor, text.len());
        self.input.insert_str(cursor, text);
        self.input_cursor = Some(cursor + text.len());
    }

    fn move_cursor_left(&mut self) {
        let cursor = self.actual_input_cursor();
        let previous = self.input[..cursor]
            .char_indices()
            .next_back()
            .map_or(0, |(index, _)| index);
        self.input_cursor = Some(previous);
        self.slash_selected = 0;
    }

    fn move_cursor_right(&mut self) {
        let cursor = self.actual_input_cursor();
        let next = self.input[cursor..]
            .chars()
            .next()
            .map_or(cursor, |character| cursor + character.len_utf8());
        self.input_cursor = Some(next);
        self.slash_selected = 0;
    }

    fn backspace_at_cursor(&mut self) {
        let cursor = self.actual_input_cursor();
        let Some((previous, _)) = self.input[..cursor].char_indices().next_back() else {
            return;
        };
        self.update_skill_bindings_for_edit(previous, cursor, 0);
        self.input.replace_range(previous..cursor, "");
        self.input_cursor = Some(previous);
    }

    fn update_skill_bindings_for_edit(&mut self, start: usize, end: usize, replacement_len: usize) {
        self.skill_bindings
            .retain(|binding| binding.end <= start || binding.start >= end);
        let removed = end - start;
        for binding in &mut self.skill_bindings {
            if binding.start >= end {
                binding.start = binding.start - removed + replacement_len;
                binding.end = binding.end - removed + replacement_len;
            }
        }
    }

    fn newer_input(&mut self) {
        let Some(index) = self.history_cursor else {
            return;
        };
        if index + 1 < self.input_history.len() {
            self.history_cursor = Some(index + 1);
            self.input.clone_from(&self.input_history[index + 1].text);
            self.skill_bindings
                .clone_from(&self.input_history[index + 1].skill_bindings);
            self.input_cursor = None;
        } else {
            self.history_cursor = None;
            self.input.clone_from(&self.history_draft.text);
            self.skill_bindings
                .clone_from(&self.history_draft.skill_bindings);
            self.input_cursor = None;
        }
    }

    fn start_completion(&mut self) {
        let end = self.actual_input_cursor();
        let completion = shell_completion::complete(&self.input[..end], &self.completion_cwd);
        match completion.candidates.as_slice() {
            [] => self.completion_popup = None,
            [candidate] => {
                self.update_skill_bindings_for_edit(completion.start, end, candidate.len());
                self.input.replace_range(completion.start..end, candidate);
                self.input_cursor = Some(completion.start + candidate.len());
                self.completion_popup = None;
                self.history_cursor = None;
            }
            _ => {
                self.completion_popup = Some(CompletionPopup {
                    start: completion.start,
                    end,
                    candidates: completion.candidates,
                    selected: 0,
                });
            }
        }
    }

    fn build_submission(&self, displayed_text: String) -> Submission {
        let mut inputs = Vec::new();
        let mut expanded = String::with_capacity(displayed_text.len());
        let mut cursor = 0;
        while cursor < displayed_text.len() {
            let next_text = self
                .pasted_texts
                .iter()
                .filter_map(|paste| {
                    displayed_text[cursor..]
                        .find(&paste.placeholder)
                        .map(|offset| (cursor + offset, paste, false))
                })
                .min_by_key(|(offset, _, _)| *offset);
            let next_image = self
                .pasted_images
                .iter()
                .filter_map(|image| {
                    displayed_text[cursor..]
                        .find(&image.placeholder)
                        .map(|offset| (cursor + offset, image))
                })
                .min_by_key(|(offset, _)| *offset);
            let next_is_image = match (next_text.as_ref(), next_image.as_ref()) {
                (Some((text_offset, _, _)), Some((image_offset, _))) => image_offset < text_offset,
                (None, Some(_)) => true,
                _ => false,
            };
            if next_is_image {
                let (offset, image) = next_image.expect("image candidate exists");
                expanded.push_str(&displayed_text[cursor..offset]);
                push_text_input(&mut inputs, std::mem::take(&mut expanded));
                inputs.push(SubmissionInput::LocalImage(image.path.clone()));
                cursor = offset + image.placeholder.len();
                continue;
            }
            let Some((offset, paste, _)) = next_text else {
                expanded.push_str(&displayed_text[cursor..]);
                break;
            };
            expanded.push_str(&displayed_text[cursor..offset]);
            expanded.push_str(&paste.text);
            cursor = offset + paste.placeholder.len();
        }
        push_text_input(&mut inputs, expanded);
        for binding in &self.skill_bindings {
            let mention = format!("${}", binding.name);
            if displayed_text.get(binding.start..binding.end) == Some(mention.as_str()) {
                inputs.push(SubmissionInput::Skill {
                    name: binding.name.clone(),
                    path: binding.path.clone(),
                });
            }
        }
        Submission {
            displayed_text,
            input: inputs,
        }
    }

    fn apply_selected_completion(&mut self) {
        let Some(popup) = self.completion_popup.take() else {
            return;
        };
        let Some(candidate) = popup.candidates.get(popup.selected) else {
            return;
        };
        self.update_skill_bindings_for_edit(popup.start, popup.end, candidate.len());
        self.input.replace_range(popup.start..popup.end, candidate);
        self.input_cursor = Some(popup.start + candidate.len());
        self.history_cursor = None;
    }

    fn refresh_skill_popup(&mut self) {
        if self.prompt.is_some() || self.mode != Mode::Editing {
            self.skill_popup = None;
            return;
        }
        let end = self.actual_input_cursor();
        let Some((start, query)) = skill_query(&self.input, end) else {
            self.skill_popup = None;
            return;
        };
        let query = query.to_ascii_lowercase();
        let candidates = self
            .skills
            .iter()
            .enumerate()
            .filter(|(_, skill)| skill.name.to_ascii_lowercase().starts_with(&query))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            self.skill_popup = None;
        } else {
            let selected = self
                .skill_popup
                .as_ref()
                .map_or(0, |popup| popup.selected.min(candidates.len() - 1));
            self.skill_popup = Some(SkillPopup {
                start,
                end,
                candidates,
                selected,
            });
            self.completion_popup = None;
        }
    }

    fn apply_selected_skill(&mut self) {
        let Some(popup) = self.skill_popup.take() else {
            return;
        };
        let Some(skill_index) = popup.candidates.get(popup.selected) else {
            return;
        };
        let replacement = format!("${} ", self.skills[*skill_index].name);
        self.update_skill_bindings_for_edit(popup.start, popup.end, replacement.len());
        self.input
            .replace_range(popup.start..popup.end, &replacement);
        self.skill_bindings.push(SkillBinding {
            start: popup.start,
            end: popup.start + replacement.len() - 1,
            name: self.skills[*skill_index].name.clone(),
            path: self.skills[*skill_index].path.clone(),
        });
        self.input_cursor = Some(popup.start + replacement.len());
        self.history_cursor = None;
    }

    pub(crate) fn clear_prompt(&mut self, request_id: &serde_json::Value) {
        if self
            .prompt
            .as_ref()
            .is_some_and(|prompt| &prompt.request_id == request_id)
        {
            self.finish_prompt();
        }
    }

    fn finish_prompt(&mut self) {
        self.prompt = None;
        self.patch_open = false;
        self.input = self.prompt_draft.take().unwrap_or_default();
        self.input_cursor = self.prompt_draft_cursor.take();
        self.skill_bindings = std::mem::take(&mut self.prompt_draft_skill_bindings);
        if let Some(picker) = self.suspended_permission_picker.take() {
            if self.threads.contains_key(&picker.target_id) {
                self.permission_picker = Some(picker);
                self.mode = Mode::Navigation;
            } else {
                self.status_line = format!(
                    "permissions picker closed: thread {} is no longer available",
                    picker.target_id
                );
            }
        }
    }

    fn prepare_subagent_request(&mut self) -> Action {
        let Some(root_id) = self.root_id.as_ref() else {
            self.status_line = "open a Main session before starting an agent".to_string();
            return Action::None;
        };
        if let Some(root_index) = self.order.iter().position(|id| id == root_id) {
            self.selected = root_index;
        }
        self.mode = Mode::Editing;
        self.input = "Start a new sub-agent for this task: ".to_string();
        self.skill_bindings.clear();
        self.input_cursor = None;
        self.history_cursor = None;
        self.scroll = u16::MAX;
        Action::SelectionChanged
    }
}

fn agent_log_lines(thread: &AgentThread, width: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for entry in &thread.log {
        lines.extend(log_entry_lines(entry, width));
        lines.push(Line::default());
    }
    lines.pop();
    lines
}

fn log_entry_lines(entry: &LogEntry, width: u16) -> Vec<Line<'static>> {
    match entry.kind {
        LogKind::User => {
            let style = Style::default().bg(user_message_background());
            let timing = entry
                .timing_label()
                .map_or_else(|| "You".to_string(), |timing| format!("You · {timing}"));
            let mut lines = vec![styled_full_line(format!(" {timing}"), width, style.bold())];
            lines.extend(
                wrapped_strings(&entry.text, width.saturating_sub(2))
                    .into_iter()
                    .map(|line| styled_full_line(format!(" {line}"), width, style)),
            );
            lines
        }
        LogKind::Agent => prefixed_log_lines("● ", &entry.text, width, Style::default()),
        LogKind::Activity => {
            prefixed_log_lines("● ", &entry.text, width, Style::default().fg(ACCENT_GREEN))
        }
    }
}

fn plain_lines(text: &str, width: u16) -> Vec<Line<'static>> {
    wrapped_strings(text, width)
        .into_iter()
        .map(Line::from)
        .collect()
}

fn patch_lines(text: &str, width: u16) -> Vec<Line<'static>> {
    text.lines()
        .flat_map(|line| {
            let style = if line.starts_with("+++") || line.starts_with("---") {
                Style::default().fg(MUTED_TEXT)
            } else if line.starts_with('+') {
                Style::default().fg(ACCENT_GREEN)
            } else if line.starts_with('-') {
                Style::default().fg(Color::Red)
            } else if line.starts_with("@@") {
                Style::default().fg(ACCENT_CYAN)
            } else if line.contains(": /") || line.starts_with("update:") {
                Style::default().bold()
            } else {
                Style::default()
            };
            wrapped_strings(line, width.saturating_sub(2))
                .into_iter()
                .map(move |line| Line::from(Span::styled(format!("  {line}"), style)))
        })
        .collect()
}

fn wrapped_strings(text: &str, width: u16) -> Vec<String> {
    let width = width.max(1) as usize;
    let lines = text
        .lines()
        .flat_map(|line| {
            if line.is_empty() {
                vec![String::new()]
            } else {
                textwrap::wrap(line, width)
                    .into_iter()
                    .map(|line| line.into_owned())
                    .collect()
            }
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        vec![String::new()]
    } else {
        lines
    }
}

fn styled_full_line(text: String, width: u16, style: Style) -> Line<'static> {
    let padding = (width as usize).saturating_sub(text.chars().count());
    Line::from(Span::styled(
        format!("{text}{}", " ".repeat(padding)),
        style,
    ))
}

fn user_message_background() -> Color {
    SURFACE_BACKGROUND
}

fn prefixed_log_lines(
    prefix: &str,
    text: &str,
    width: u16,
    prefix_style: Style,
) -> Vec<Line<'static>> {
    let prefix_width = prefix.chars().count() as u16;
    wrapped_strings(text, width.saturating_sub(prefix_width))
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            let marker = if index == 0 { prefix } else { "  " };
            Line::from(vec![
                Span::styled(marker.to_string(), prefix_style),
                Span::from(line),
            ])
        })
        .collect()
}

fn centered_session_area(outer: Rect, candidate_count: usize) -> Rect {
    let width = outer.width.saturating_sub(4).min(92);
    let desired_height = (candidate_count.saturating_add(3)).min(u16::MAX as usize) as u16;
    let height = desired_height.min(outer.height.saturating_sub(4)).max(3);
    Rect::new(
        outer.x + outer.width.saturating_sub(width) / 2,
        outer.y + outer.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

fn centered_info_area(outer: Rect) -> Rect {
    let width = outer.width.saturating_sub(2).clamp(1, 100);
    let height = outer.height.saturating_sub(2).clamp(1, 14);
    Rect::new(
        outer.x + outer.width.saturating_sub(width) / 2,
        outer.y + outer.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

fn nonempty_or<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.is_empty() { fallback } else { value }
}

fn composer_viewport(text: &str, cursor: usize, width: u16, height: u16) -> (u16, u16, u16) {
    let width = width.max(1) as usize;
    let height = height.max(1) as usize;
    let cursor = cursor.min(text.len());
    let lines = wrap_composer_input(&text[..cursor], width);
    let line_count = lines.len().max(1);
    let scroll = line_count.saturating_sub(height);
    let last_line = lines.last().map_or("", AsRef::as_ref);
    let cursor_column = textwrap::core::display_width(last_line).min(width);
    let cursor_row = line_count.saturating_sub(1).saturating_sub(scroll);
    (
        scroll.min(u16::MAX as usize) as u16,
        cursor_column.min(u16::MAX as usize) as u16,
        cursor_row.min(u16::MAX as usize) as u16,
    )
}

/// Hard-wraps composer input without textwrap's word-boundary whitespace trimming.
///
/// The trailing empty line at an exact boundary is intentional: it is the cell
/// where the terminal cursor belongs after the last visible column is occupied.
fn wrap_composer_input(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = vec![String::new()];
    let mut column: usize = 0;

    for character in text.chars() {
        if character == '\n' {
            lines.push(String::new());
            column = 0;
            continue;
        }

        let character_width = textwrap::core::display_width(&character.to_string());
        if column > 0 && column.saturating_add(character_width) > width {
            lines.push(String::new());
            column = 0;
        }
        lines
            .last_mut()
            .expect("composer wrapper always has a line")
            .push(character);
        column = column.saturating_add(character_width);

        if column >= width {
            lines.push(String::new());
            column = 0;
        }
    }

    lines
}

fn relative_time(timestamp: i64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs() as i64);
    let elapsed = now.saturating_sub(timestamp).max(0) as u64;
    match elapsed {
        0..=59 => "just now".to_string(),
        60..=3_599 => format!("{}m ago", elapsed / 60),
        3_600..=86_399 => format!("{}h ago", elapsed / 3_600),
        _ => format!("{}d ago", elapsed / 86_400),
    }
}

fn truncate_line(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    if max_chars <= 1 {
        return "…".chars().take(max_chars).collect();
    }
    format!("{}…", text.chars().take(max_chars - 1).collect::<String>())
}

fn append_children(order: &mut Vec<String>, parent: &str, threads: &HashMap<String, AgentThread>) {
    let mut children = threads
        .values()
        .filter(|thread| thread.parent_id.as_deref() == Some(parent) && thread.status != "closed")
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
        "working" => Style::default().fg(ACCENT_GREEN),
        "error" => Style::default().fg(Color::Red),
        "closed" => Style::default().fg(MUTED_TEXT),
        _ => Style::default(),
    }
}

fn is_full_access_profile(profile: &str) -> bool {
    profile.eq_ignore_ascii_case("full") || profile.eq_ignore_ascii_case("full-access")
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
