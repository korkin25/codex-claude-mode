use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

const MAX_ENTRIES: usize = 2_000;
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EditorKind {
    Terminal,
    VsCode,
    Cursor,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BrowserAction {
    None,
    Close,
    OpenEditor { editor: EditorKind, path: PathBuf },
}

#[derive(Debug)]
struct Entry {
    path: PathBuf,
    depth: usize,
    directory: bool,
}

#[derive(Debug)]
struct Viewer {
    path: PathBuf,
    lines: Vec<String>,
    scroll: usize,
    error: Option<String>,
}

pub(crate) struct ProjectBrowser {
    root: PathBuf,
    expanded: HashSet<PathBuf>,
    selected: usize,
    tree_scroll: usize,
    viewer: Option<Viewer>,
}

impl ProjectBrowser {
    pub(crate) fn open(root: PathBuf) -> Self {
        let root = root.canonicalize().unwrap_or(root);
        Self {
            expanded: HashSet::from([root.clone()]),
            root,
            selected: 0,
            tree_scroll: 0,
            viewer: None,
        }
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> BrowserAction {
        if let Some(viewer) = self.viewer.as_mut() {
            let page = 20;
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Left | KeyCode::Char('h') => {
                    self.viewer = None
                }
                KeyCode::Up | KeyCode::Char('k') => viewer.scroll = viewer.scroll.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => {
                    viewer.scroll = viewer.scroll.saturating_add(1)
                }
                KeyCode::PageUp => viewer.scroll = viewer.scroll.saturating_sub(page),
                KeyCode::PageDown => viewer.scroll = viewer.scroll.saturating_add(page),
                KeyCode::Home | KeyCode::Char('g') => viewer.scroll = 0,
                KeyCode::End | KeyCode::Char('G') => viewer.scroll = usize::MAX,
                KeyCode::Char('e') => return Self::editor(EditorKind::Terminal, &viewer.path),
                KeyCode::Char('v') => return Self::editor(EditorKind::VsCode, &viewer.path),
                KeyCode::Char('c') => return Self::editor(EditorKind::Cursor, &viewer.path),
                _ => {}
            }
            return BrowserAction::None;
        }

        let entries = self.entries();
        self.selected = self.selected.min(entries.len().saturating_sub(1));
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('t') => return BrowserAction::Close,
            KeyCode::Up | KeyCode::Char('k') => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = (self.selected + 1).min(entries.len().saturating_sub(1));
            }
            KeyCode::Home | KeyCode::Char('g') => self.selected = 0,
            KeyCode::End | KeyCode::Char('G') => self.selected = entries.len().saturating_sub(1),
            KeyCode::Left | KeyCode::Char('h') => self.collapse_or_parent(&entries),
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Enter => self.expand_or_open(&entries),
            KeyCode::Char('e') => return self.editor_for_selected(&entries, EditorKind::Terminal),
            KeyCode::Char('v') => return self.editor_for_selected(&entries, EditorKind::VsCode),
            KeyCode::Char('c') => return self.editor_for_selected(&entries, EditorKind::Cursor),
            _ => {}
        }
        BrowserAction::None
    }

    fn editor(editor: EditorKind, path: &Path) -> BrowserAction {
        BrowserAction::OpenEditor {
            editor,
            path: path.to_path_buf(),
        }
    }

    fn editor_for_selected(&self, entries: &[Entry], editor: EditorKind) -> BrowserAction {
        entries
            .get(self.selected)
            .map_or(BrowserAction::None, |entry| {
                Self::editor(editor, &entry.path)
            })
    }

    fn collapse_or_parent(&mut self, entries: &[Entry]) {
        let Some(entry) = entries.get(self.selected) else {
            return;
        };
        if entry.directory && self.expanded.remove(&entry.path) {
            return;
        }
        let Some(parent) = entry.path.parent() else {
            return;
        };
        if let Some(index) = entries
            .iter()
            .position(|candidate| candidate.path == parent)
        {
            self.selected = index;
        }
    }

    fn expand_or_open(&mut self, entries: &[Entry]) {
        let Some(entry) = entries.get(self.selected) else {
            return;
        };
        if entry.directory {
            self.expanded.insert(entry.path.clone());
        } else {
            self.viewer = Some(Viewer::load(entry.path.clone()));
        }
    }

    fn entries(&self) -> Vec<Entry> {
        let mut entries = vec![Entry {
            path: self.root.clone(),
            depth: 0,
            directory: true,
        }];
        self.append_children(&self.root, 1, &mut entries);
        entries
    }

    fn append_children(&self, directory: &Path, depth: usize, entries: &mut Vec<Entry>) {
        if !self.expanded.contains(directory) || entries.len() >= MAX_ENTRIES {
            return;
        }
        let Ok(read_dir) = fs::read_dir(directory) else {
            return;
        };
        let mut children = read_dir
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.starts_with('.') || matches!(name.as_ref(), "target" | "node_modules") {
                    return None;
                }
                let file_type = entry.file_type().ok()?;
                Some(Entry {
                    path: entry.path(),
                    depth,
                    directory: file_type.is_dir(),
                })
            })
            .collect::<Vec<_>>();
        children.sort_by(|left, right| {
            right
                .directory
                .cmp(&left.directory)
                .then_with(|| left.path.file_name().cmp(&right.path.file_name()))
        });
        for child in children {
            if entries.len() >= MAX_ENTRIES {
                break;
            }
            let recurse_path = child.path.clone();
            let recurse = child.directory;
            entries.push(child);
            if recurse {
                self.append_children(&recurse_path, depth + 1, entries);
            }
        }
    }

    pub(crate) fn render(&mut self, frame: &mut Frame) {
        if self.viewer.is_some() {
            self.render_viewer(frame);
        } else {
            self.render_tree(frame);
        }
    }

    fn render_tree(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let entries = self.entries();
        self.selected = self.selected.min(entries.len().saturating_sub(1));
        let inner_height = area.height.saturating_sub(2) as usize;
        if self.selected < self.tree_scroll {
            self.tree_scroll = self.selected;
        }
        if self.selected >= self.tree_scroll.saturating_add(inner_height) {
            self.tree_scroll = self.selected.saturating_sub(inner_height.saturating_sub(1));
        }
        let lines = entries
            .iter()
            .enumerate()
            .skip(self.tree_scroll)
            .take(inner_height)
            .map(|(index, entry)| {
                let name = if entry.depth == 0 {
                    entry.path.display().to_string()
                } else {
                    entry
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                };
                let marker = if entry.directory {
                    if self.expanded.contains(&entry.path) {
                        "▾ "
                    } else {
                        "▸ "
                    }
                } else {
                    "  "
                };
                let text = format!("{}{}{}", "  ".repeat(entry.depth), marker, name);
                let style = if index == self.selected {
                    Style::default().bg(Color::DarkGray).bold()
                } else if entry.directory {
                    Style::default().cyan()
                } else {
                    Style::default()
                };
                Line::from(Span::styled(text, style))
            })
            .collect::<Vec<_>>();
        frame.render_widget(Clear, area);
        frame.render_widget(Paragraph::new(Text::from(lines)).block(Block::new().borders(Borders::ALL)
            .border_style(Style::default().cyan()).title(" Project tree ")
            .title_bottom(" ↑↓/jk move · →/l/Enter open · ←/h parent · e vim · v VS Code · c Cursor · q close ")), area);
    }

    fn render_viewer(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let viewer = self.viewer.as_mut().expect("viewer exists");
        let height = area.height.saturating_sub(2) as usize;
        let max_scroll = viewer.lines.len().saturating_sub(height);
        viewer.scroll = viewer.scroll.min(max_scroll);
        let width = viewer.lines.len().max(1).to_string().len();
        let extension = viewer
            .path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        let lines = if let Some(error) = &viewer.error {
            vec![Line::from(error.as_str().red())]
        } else {
            viewer
                .lines
                .iter()
                .enumerate()
                .skip(viewer.scroll)
                .take(height)
                .map(|(index, line)| {
                    let mut spans = vec![Span::styled(
                        format!("{:>width$} │ ", index + 1),
                        Style::default().dark_gray(),
                    )];
                    spans.extend(highlight_line(line, extension));
                    Line::from(spans)
                })
                .collect()
        };
        let title = format!(" {} ", viewer.path.display());
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(Text::from(lines)).block(
                Block::new()
                    .borders(Borders::ALL)
                    .border_style(Style::default().cyan())
                    .title(title)
                    .title_bottom(
                        " ↑↓/jk · PgUp/PgDn · g/G · e vim · v VS Code · c Cursor · q back ",
                    ),
            ),
            area,
        );
    }
}

impl Viewer {
    fn load(path: PathBuf) -> Self {
        let result = fs::metadata(&path).and_then(|metadata| {
            if metadata.len() > MAX_FILE_BYTES {
                return Err(std::io::Error::other("file is larger than 2 MiB"));
            }
            fs::read_to_string(&path)
        });
        match result {
            Ok(text) => Self {
                path,
                lines: text.lines().map(ToOwned::to_owned).collect(),
                scroll: 0,
                error: None,
            },
            Err(error) => Self {
                path,
                lines: Vec::new(),
                scroll: 0,
                error: Some(error.to_string()),
            },
        }
    }
}

fn highlight_line(line: &str, extension: &str) -> Vec<Span<'static>> {
    let trimmed = line.trim_start();
    let comment = match extension {
        "rs" | "js" | "ts" | "tsx" | "jsx" | "c" | "h" | "cpp" | "java" => {
            trimmed.starts_with("//")
        }
        "py" | "rb" | "sh" | "bash" | "zsh" | "toml" | "yaml" | "yml" => trimmed.starts_with('#'),
        _ => false,
    };
    if comment {
        return vec![Span::styled(
            line.to_string(),
            Style::default().dark_gray().italic(),
        )];
    }
    let keywords: &[&str] = match extension {
        "rs" => &[
            "fn", "let", "mut", "pub", "struct", "enum", "impl", "use", "mod", "match", "if",
            "else", "return",
        ],
        "py" => &[
            "def", "class", "import", "from", "if", "else", "elif", "return", "for", "while", "in",
        ],
        "js" | "ts" | "tsx" | "jsx" => &[
            "function", "const", "let", "class", "import", "export", "return", "if", "else",
        ],
        _ => &[],
    };
    let mut spans = Vec::new();
    for part in line.split_inclusive(|character: char| character.is_whitespace()) {
        let word = part.trim_end();
        let suffix = &part[word.len()..];
        let style = if keywords.contains(&word) {
            Style::default().magenta().bold()
        } else if word.starts_with(['\"', '\'']) {
            Style::default().green()
        } else {
            Style::default()
        };
        spans.push(Span::styled(word.to_string(), style));
        if !suffix.is_empty() {
            spans.push(Span::raw(suffix.to_string()));
        }
    }
    spans
}

#[cfg(test)]
#[path = "project_tree_tests.rs"]
mod tests;
