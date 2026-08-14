mod backend;
mod clipboard;
mod command;
mod editor;
mod model;
mod project_tree;
mod prompt;
mod session;
mod shell_completion;
mod ui;
mod version;

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;

use std::collections::HashMap;
use std::collections::HashSet;
use std::env;
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::time::Duration;
use std::time::Instant;

use anyhow::Context;
use anyhow::Result;
use backend::Backend;
use backend::BackendEvent;
use clap::Parser;
use crossterm::event;
use crossterm::event::DisableBracketedPaste;
use crossterm::event::DisableMouseCapture;
use crossterm::event::EnableBracketedPaste;
use crossterm::event::EnableMouseCapture;
use crossterm::event::Event;
use crossterm::execute;
use crossterm::terminal::EnterAlternateScreen;
use crossterm::terminal::LeaveAlternateScreen;
use crossterm::terminal::disable_raw_mode;
use crossterm::terminal::enable_raw_mode;
use editor::ExternalEditor;
use model::AgentThread;
use prompt::PromptResolution;
use prompt::ServerPrompt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use serde_json::Value;
use serde_json::json;
use session::candidates_from_list;
use ui::Action;
use ui::PermissionChoice;
use ui::SessionSelection;
use ui::SkillChoice;
use ui::Submission;
use ui::SubmissionInput;
use ui::Workspace;

const SOURCE_KINDS: &[&str] = &[
    "cli",
    "vscode",
    "exec",
    "appServer",
    "subAgent",
    "subAgentReview",
    "subAgentCompact",
    "subAgentThreadSpawn",
    "subAgentOther",
    "unknown",
];

#[derive(Parser)]
#[command(version, about, disable_help_flag = true)]
struct Args {
    /// Installed Codex executable to use as the backend.
    #[arg(long, env = "CODEX_BIN", default_value = "codex")]
    codex: PathBuf,

    /// Codex home used by the installed backend.
    #[arg(long, env = "CODEX_HOME")]
    codex_home: Option<PathBuf>,

    /// Workspace whose latest Main/sub-agent tree should be opened.
    #[arg(long)]
    cwd: Option<PathBuf>,

    /// Open a specific root thread instead of the newest root in this directory.
    #[arg(long)]
    thread: Option<String>,

    /// Verify protocol compatibility without opening the TUI.
    #[arg(long)]
    check_backend: bool,

    /// Show wrapper options followed by the installed Codex help.
    #[arg(long = "help", short = 'h', action = clap::ArgAction::SetTrue)]
    combined_help: bool,
}

#[derive(Debug)]
enum Pending {
    Initialize,
    List {
        generation: u64,
    },
    Descendants {
        root_id: String,
        generation: u64,
    },
    OpenThread {
        thread_id: String,
        resume_cwd: PathBuf,
    },
    Read(String),
    Start,
    ResumeAndSend {
        target_id: String,
        submission: Submission,
    },
    Turn,
    Interrupt,
    Command(String),
    Skills {
        announce: bool,
    },
    Permissions {
        target_id: String,
        requested: Option<String>,
    },
    PermissionUpdate,
}

const MAX_LIST_PAGES: usize = 100;
const MAX_LIST_ITEMS: usize = 20_000;

struct ListChain {
    generation: u64,
    all_workspaces: bool,
    pages: usize,
    cursors: HashSet<String>,
    ids: HashSet<String>,
    threads: Vec<Value>,
}

struct DescendantChain {
    generation: u64,
    root_id: String,
    pages: usize,
    cursors: HashSet<String>,
    ids: HashSet<String>,
}

struct App {
    backend: Backend,
    workspace: Workspace,
    pending: HashMap<u64, Pending>,
    loaded_history: HashSet<String>,
    live_items: HashMap<(String, String), Value>,
    preferred_root: Option<String>,
    session_decided: bool,
    starting_new_session: bool,
    cwd: PathBuf,
    active_session_cwd: PathBuf,
    last_refresh: Instant,
    permission_profiles: HashMap<String, String>,
    codex: PathBuf,
    codex_home: PathBuf,
    update_result: Option<Receiver<std::result::Result<String, String>>>,
    clipboard_images: clipboard::ClipboardImages,
    clipboard_capture: Option<clipboard::ClipboardCapture>,
    clipboard_target: Option<ui::ClipboardTarget>,
    list_generation: u64,
    list_chain: Option<ListChain>,
    descendants_generation: u64,
    descendants_chain: Option<DescendantChain>,
    codex_home_was_empty: bool,
}

fn main() -> Result<()> {
    let (wrapper_args, codex_args) = split_args(env::args_os());
    let args = Args::parse_from(wrapper_args);
    if args.combined_help {
        print_combined_help(&args.codex, &codex_args)?;
        return Ok(());
    }
    let codex_home = args.codex_home.unwrap_or_else(default_codex_home);
    let codex_home_was_empty = directory_is_empty_or_missing(&codex_home);
    let cwd = args
        .cwd
        .unwrap_or(env::current_dir().context("failed to read cwd")?);
    let codex_version = version::read(&args.codex, &codex_home);
    let mut backend = Backend::spawn(&args.codex, &codex_home, &codex_args)?;
    if args.check_backend {
        return check_backend(&mut backend, &args.codex, &codex_home, &cwd);
    }
    let initialize_id = backend.initialize()?;
    let mut workspace = Workspace::new();
    workspace.set_completion_cwd(cwd.clone());
    workspace.set_codex_home(codex_home.clone(), codex_home_was_empty);
    workspace.set_codex_versions(codex_version.current, codex_version.latest);
    let mut app = App {
        backend,
        workspace,
        pending: HashMap::from([(initialize_id, Pending::Initialize)]),
        loaded_history: HashSet::new(),
        live_items: HashMap::new(),
        session_decided: args.thread.is_some(),
        starting_new_session: false,
        preferred_root: args.thread,
        active_session_cwd: cwd.clone(),
        cwd,
        last_refresh: Instant::now(),
        permission_profiles: HashMap::new(),
        codex: args.codex,
        codex_home,
        update_result: None,
        clipboard_images: clipboard::ClipboardImages::new(),
        clipboard_capture: None,
        clipboard_target: None,
        list_generation: 0,
        list_chain: None,
        descendants_generation: 0,
        descendants_chain: None,
        codex_home_was_empty,
    };
    run_terminal(&mut app)
}

fn split_args(args: impl IntoIterator<Item = OsString>) -> (Vec<OsString>, Vec<OsString>) {
    let mut args = args.into_iter();
    let program = args.next().unwrap_or_else(|| "codex-claude-mode".into());
    let mut wrapper = vec![program];
    let mut codex = Vec::new();
    let mut passthrough = false;
    let mut takes_value = false;
    for arg in args {
        if passthrough {
            codex.push(arg);
            continue;
        }
        if takes_value {
            wrapper.push(arg);
            takes_value = false;
            continue;
        }
        if arg == "--" {
            passthrough = true;
            continue;
        }
        let text = arg.to_string_lossy();
        let wrapper_value = ["--codex", "--codex-home", "--cwd", "--thread"];
        if wrapper_value.contains(&text.as_ref()) {
            takes_value = true;
            wrapper.push(arg);
        } else if wrapper_value
            .iter()
            .any(|name| text.starts_with(&format!("{name}=")))
            || matches!(
                text.as_ref(),
                "--check-backend" | "-h" | "--help" | "-V" | "--version"
            )
        {
            wrapper.push(arg);
        } else {
            codex.push(arg);
        }
    }
    (wrapper, codex)
}

fn print_combined_help(codex: &std::path::Path, codex_args: &[OsString]) -> Result<()> {
    use clap::CommandFactory;

    Args::command().print_help()?;
    println!("\n\nInstalled Codex options ({}):\n", codex.display());
    let status = Command::new(codex)
        .args(codex_args)
        .arg("--help")
        .status()
        .with_context(|| format!("failed to run {} --help", codex.display()))?;
    if !status.success() {
        anyhow::bail!("{} --help exited with {status}", codex.display());
    }
    Ok(())
}

fn default_codex_home() -> PathBuf {
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    codex_home_from_home(home)
}

fn codex_home_from_home(home: PathBuf) -> PathBuf {
    home.join(".codex")
}

fn check_backend(
    backend: &mut Backend,
    codex: &std::path::Path,
    codex_home: &std::path::Path,
    cwd: &std::path::Path,
) -> Result<()> {
    let initialize_id = backend.initialize()?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        let Some(event) = backend.recv_timeout(Duration::from_millis(250)) else {
            continue;
        };
        match event {
            BackendEvent::Message(message)
                if message.get("id").and_then(Value::as_u64) == Some(initialize_id) =>
            {
                if let Some(error) = message.get("error") {
                    anyhow::bail!("initialize failed: {error}");
                }
                backend.initialized()?;
                let list_id = backend.request("thread/list", list_params(Some(cwd), None))?;
                loop {
                    match backend.recv_timeout(Duration::from_secs(10)) {
                        Some(BackendEvent::Message(message))
                            if message.get("id").and_then(Value::as_u64) == Some(list_id) =>
                        {
                            if let Some(error) = message.get("error") {
                                anyhow::bail!("thread/list failed: {error}");
                            }
                            let count = message
                                .pointer("/result/data")
                                .and_then(Value::as_array)
                                .map_or(0, Vec::len);
                            println!(
                                "compatible: {} app-server; CODEX_HOME={}; threads={count}",
                                codex.display(),
                                codex_home.display()
                            );
                            return Ok(());
                        }
                        Some(BackendEvent::Stderr(line)) => eprintln!("{line}"),
                        Some(BackendEvent::Exited) | None => {
                            anyhow::bail!("app-server exited during thread/list")
                        }
                        _ => {}
                    }
                }
            }
            BackendEvent::Stderr(line) => eprintln!("{line}"),
            BackendEvent::Exited => anyhow::bail!("app-server exited during initialize"),
            _ => {}
        }
    }
    anyhow::bail!("app-server initialize timed out")
}

fn run_terminal(app: &mut App) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    enter_terminal(&mut stdout)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;
    let result = run_event_loop(app, &mut terminal);
    disable_raw_mode()?;
    leave_terminal(terminal.backend_mut())?;
    terminal.show_cursor()?;
    result
}

fn enter_terminal(writer: &mut impl io::Write) -> Result<()> {
    execute!(
        writer,
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste
    )?;
    Ok(())
}

fn leave_terminal(writer: &mut impl io::Write) -> Result<()> {
    execute!(
        writer,
        DisableBracketedPaste,
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    Ok(())
}

fn run_event_loop(
    app: &mut App,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> Result<()> {
    loop {
        while let Some(event) = app.backend.try_recv() {
            app.handle_backend_event(event)?;
        }
        app.poll_codex_update();
        app.poll_clipboard_capture();
        if app.session_decided
            && !app.starting_new_session
            && app.last_refresh.elapsed() >= Duration::from_secs(2)
            && app
                .pending
                .values()
                .all(|pending| !matches!(pending, Pending::List { .. }))
        {
            app.request_list(false, None)?;
        }
        terminal.draw(|frame| app.workspace.render(frame))?;
        if !event::poll(Duration::from_millis(50))? {
            continue;
        }
        let action = match event::read()? {
            Event::Key(key) => app.workspace.handle_key(key),
            Event::Mouse(mouse) => app.workspace.handle_mouse(mouse),
            Event::Paste(text) => app.workspace.handle_paste(text),
            Event::Resize(_, _) | Event::FocusGained | Event::FocusLost => Action::None,
        };
        match action {
            Action::Quit => return Ok(()),
            Action::Submit(submission) => app.submit(submission)?,
            Action::PasteImage => app.start_clipboard_capture(),
            Action::SelectionChanged => app.read_selected()?,
            Action::ResolvePrompt(resolution) => app.resolve_prompt(resolution)?,
            Action::Interrupt => app.interrupt_selected()?,
            Action::SessionSelected(selection) => app.select_session(selection)?,
            Action::ChooseSession => app.choose_session()?,
            Action::NewSession => app.select_session(None)?,
            Action::UpdateCodex => app.start_codex_update(),
            Action::PermissionSelected {
                target_id,
                profile_id,
            } => app.select_permission(target_id, profile_id)?,
            Action::OpenTerminalEditor { path, line, column } => {
                open_external_editor(app, terminal, ExternalEditor::Terminal, &path, line, column)?
            }
            Action::OpenVsCode { path, line, column } => {
                open_external_editor(app, terminal, ExternalEditor::VsCode, &path, line, column)?
            }
            Action::OpenCursor { path, line, column } => {
                open_external_editor(app, terminal, ExternalEditor::Cursor, &path, line, column)?
            }
            Action::None => {}
        }
    }
}

fn open_external_editor(
    app: &mut App,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    external_editor: ExternalEditor,
    requested: &std::path::Path,
    line: usize,
    column: usize,
) -> Result<()> {
    let project_root = app
        .workspace
        .selected_thread()
        .and_then(|thread| (!thread.cwd.is_empty()).then(|| PathBuf::from(&thread.cwd)))
        .unwrap_or_else(|| app.cwd.clone());
    let result = (|| -> Result<()> {
        let path = editor::resolve_project_file(&project_root, requested)?;
        let line = u32::try_from(line).context("editor line number is too large")?;
        let column = u32::try_from(column).context("editor column number is too large")?;
        let command = editor::command_for(external_editor, &path, Some(line), Some(column))?;
        if external_editor == ExternalEditor::Terminal {
            suspend_terminal(terminal)?;
            let editor_result = editor::run_terminal(&command);
            let resume_result = resume_terminal(terminal);
            resume_result?;
            editor_result?;
        } else {
            editor::spawn_gui(&command)?;
        }
        Ok(())
    })();
    app.workspace.status_line = match result {
        Ok(()) => format!("opened {}", requested.display()),
        Err(error) => format!("could not open {}: {error:#}", requested.display()),
    };
    Ok(())
}

fn suspend_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    if let Err(error) = suspend_terminal_features(terminal.backend_mut()) {
        let _ = enable_raw_mode();
        return Err(error);
    }
    if let Err(error) = terminal.show_cursor() {
        let _ = resume_terminal(terminal);
        return Err(error.into());
    }
    Ok(())
}

fn resume_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    enable_raw_mode()?;
    resume_terminal_features(terminal.backend_mut())?;
    terminal.clear()?;
    Ok(())
}

fn suspend_terminal_features(writer: &mut impl io::Write) -> Result<()> {
    execute!(
        writer,
        DisableBracketedPaste,
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    Ok(())
}

fn resume_terminal_features(writer: &mut impl io::Write) -> Result<()> {
    execute!(
        writer,
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste
    )?;
    Ok(())
}

impl App {
    fn start_clipboard_capture(&mut self) {
        if self.clipboard_capture.is_some() {
            self.workspace
                .set_clipboard_notice("already reading clipboard…");
            return;
        }
        let Some(target) = self.workspace.clipboard_target() else {
            self.workspace
                .set_clipboard_notice("close the current overlay before pasting");
            return;
        };
        self.clipboard_target = Some(target);
        self.clipboard_capture = Some(clipboard::ClipboardImages::capture_in_background());
        self.workspace.set_clipboard_notice("reading clipboard…");
    }

    fn poll_clipboard_capture(&mut self) {
        let Some(receiver) = self.clipboard_capture.as_ref() else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.clipboard_capture = None;
                self.clipboard_target = None;
                self.workspace
                    .set_clipboard_notice("clipboard worker stopped");
                return;
            }
        };
        self.clipboard_capture = None;
        let target = self.clipboard_target.take();
        let target_is_current = target
            .as_ref()
            .is_some_and(|target| self.workspace.clipboard_target().as_ref() == Some(target));
        match result {
            Ok(clipboard::CapturedClipboard::Image(captured)) => {
                if !target_is_current {
                    self.workspace
                        .set_clipboard_notice("clipboard changed; paste cancelled");
                    return;
                }
                match self.clipboard_images.store(captured) {
                    Ok(image) => {
                        self.workspace
                            .attach_image(image.path, image.format, image.size);
                        self.workspace.set_clipboard_notice("image pasted");
                    }
                    Err(error) => self.workspace.set_clipboard_notice(error.to_string()),
                }
            }
            Ok(clipboard::CapturedClipboard::Text(text)) => {
                if target.is_some_and(|target| self.workspace.insert_clipboard_text(&target, text))
                {
                    self.workspace.set_clipboard_notice("text pasted");
                } else {
                    self.workspace
                        .set_clipboard_notice("clipboard changed; paste cancelled");
                }
            }
            Err(error) => self.workspace.set_clipboard_notice(error.to_string()),
        }
    }

    fn start_codex_update(&mut self) {
        if self.update_result.is_some() {
            return;
        }
        let codex = self.codex.clone();
        let codex_home = self.codex_home.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result =
                version::run_update(&codex, &codex_home).map_err(|error| error.to_string());
            let _ = sender.send(result);
        });
        self.update_result = Some(receiver);
        self.workspace.codex_update_started();
    }

    fn poll_codex_update(&mut self) {
        let Some(receiver) = self.update_result.as_ref() else {
            return;
        };
        let Ok(result) = receiver.try_recv() else {
            return;
        };
        self.update_result = None;
        let message = match result {
            Ok(output) => output,
            Err(error) => format!("Codex update failed: {error}"),
        };
        self.workspace.codex_update_finished(message);
        let versions = version::read(&self.codex, &self.codex_home);
        self.workspace
            .set_codex_versions(versions.current, versions.latest);
    }

    fn handle_backend_event(&mut self, event: BackendEvent) -> Result<()> {
        match event {
            BackendEvent::Message(message) => self.handle_message(message),
            BackendEvent::Stderr(line) => {
                self.workspace.status_line = format!("app-server: {line}");
                Ok(())
            }
            BackendEvent::Exited => anyhow::bail!("installed Codex app-server exited"),
        }
    }

    fn handle_message(&mut self, message: Value) -> Result<()> {
        if let Some(id) = message.get("id").and_then(Value::as_u64)
            && let Some(pending) = self.pending.remove(&id)
        {
            return self.handle_response(pending, &message);
        }
        if message.get("method").is_some() && message.get("id").is_some() {
            let method = message
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or("request");
            let related_item = message
                .get("params")
                .and_then(|params| {
                    let thread_id = params.get("threadId")?.as_str()?;
                    let item_id = params.get("itemId")?.as_str()?;
                    self.live_items
                        .get(&(thread_id.to_string(), item_id.to_string()))
                })
                .cloned();
            match ServerPrompt::from_request_with_item(&message, related_item.as_ref()) {
                Ok(prompt) => {
                    let thread_id = prompt.thread_id.clone();
                    if let Err(prompt) = self.workspace.set_prompt(prompt) {
                        self.workspace.status_line =
                            "another interactive request is already pending".to_string();
                        return self
                            .backend
                            .reject_server_request(&prompt.request_id, "concurrent request");
                    }
                    if let Some(thread) = self.workspace.threads.get_mut(&thread_id) {
                        thread.push_activity_message(format!("Action required: {method}"));
                    }
                    return Ok(());
                }
                Err(error) => {
                    self.workspace.status_line = error;
                    return self.backend.reject_server_request(&message["id"], method);
                }
            }
        }
        if let Some(method) = message.get("method").and_then(Value::as_str) {
            self.handle_notification(method, message.get("params").unwrap_or(&Value::Null))?;
        }
        Ok(())
    }

    fn handle_response(&mut self, pending: Pending, message: &Value) -> Result<()> {
        if let Some(error) = message.get("error") {
            if matches!(&pending, Pending::Start) {
                self.starting_new_session = false;
                self.session_decided = false;
                self.request_list(false, None)?;
            } else if let Pending::OpenThread { thread_id, .. } = pending {
                // Keep the explicit-thread failure terminal until the user
                // deliberately chooses another session.  Marking session
                // selection undecided here would make the event loop
                // immediately issue thread/list and overwrite this error,
                // making a failed --thread look like a normal picker launch.
                self.session_decided = true;
                self.preferred_root = None;
                self.workspace.status_line = format!("could not open thread {thread_id}: {error}");
                return Ok(());
            } else if let Pending::List { generation } = &pending {
                if self
                    .list_chain
                    .as_ref()
                    .is_some_and(|chain| chain.generation == *generation)
                {
                    self.list_chain = None;
                    self.workspace.status_line = format!("could not list sessions: {error}");
                }
                return Ok(());
            } else if let Pending::Descendants {
                root_id,
                generation,
            } = &pending
            {
                if self.descendants_chain.as_ref().is_some_and(|chain| {
                    chain.generation == *generation && chain.root_id == *root_id
                }) {
                    self.descendants_chain = None;
                    self.workspace.status_line =
                        format!("could not list sub-agents for {root_id}: {error}");
                }
                return Ok(());
            }
            self.workspace.status_line = format!("app-server error: {error}");
            return Ok(());
        }
        let result = message.get("result").unwrap_or(&Value::Null);
        match pending {
            Pending::Initialize => {
                self.backend.initialized()?;
                if let Some(user_agent) = result.get("userAgent").and_then(Value::as_str) {
                    self.workspace.set_backend_user_agent(user_agent);
                }
                self.workspace.status_line = format!(
                    "connected to {}",
                    result
                        .get("userAgent")
                        .and_then(Value::as_str)
                        .unwrap_or("Codex")
                );
                self.request_skills(false, false)?;
                if let Some(thread_id) = self.preferred_root.clone() {
                    let resume_cwd = match validated_resume_cwd(&self.cwd) {
                        Ok(cwd) => cwd,
                        Err(error) => {
                            self.workspace.status_line =
                                format!("cannot open thread {thread_id}: {error}");
                            return Ok(());
                        }
                    };
                    let id = self.backend.request(
                        "thread/resume",
                        thread_resume_params(&thread_id, &resume_cwd),
                    )?;
                    self.pending.insert(
                        id,
                        Pending::OpenThread {
                            thread_id,
                            resume_cwd,
                        },
                    );
                } else {
                    self.request_list(false, None)?;
                }
            }
            Pending::List { generation } => self.apply_list(result, generation)?,
            Pending::Descendants {
                root_id,
                generation,
            } => self.apply_descendants(&root_id, generation, result)?,
            Pending::OpenThread {
                thread_id,
                resume_cwd,
            } => {
                let thread = result.get("thread").unwrap_or(result);
                if AgentThread::from_json(thread).is_none() {
                    self.session_decided = true;
                    self.preferred_root = None;
                    self.workspace.status_line = format!("thread {thread_id} was not found");
                } else {
                    self.active_session_cwd = resume_cwd;
                    self.upsert_thread(thread);
                    self.preferred_root = Some(thread_id.clone());
                    self.loaded_history.insert(thread_id.clone());
                    self.workspace.rebuild_tree(Some(&thread_id));
                    self.workspace.status_line = format!("opened {thread_id}");
                    self.request_descendants(None)?;
                }
            }
            Pending::Read(thread_id) => {
                if let Some(thread) = result.get("thread") {
                    self.upsert_thread(thread);
                    self.loaded_history.insert(thread_id.clone());
                    self.workspace.rebuild_tree(self.preferred_root.as_deref());
                    self.workspace.status_line = format!("loaded {thread_id}");
                }
            }
            Pending::Start => {
                if let Some(thread) = result.get("thread") {
                    self.active_session_cwd = self.cwd.clone();
                    self.starting_new_session = false;
                    self.workspace.clear_session_picker();
                    self.upsert_thread(thread);
                    self.preferred_root = thread.get("id").and_then(Value::as_str).map(|id| {
                        self.loaded_history.insert(id.to_string());
                        id.to_string()
                    });
                    self.workspace.rebuild_tree(self.preferred_root.as_deref());
                    self.workspace.status_line = "new Main thread ready".to_string();
                }
            }
            Pending::ResumeAndSend {
                target_id,
                submission,
            } => self.start_turn(&target_id, &submission.input)?,
            Pending::Turn => {}
            Pending::Interrupt => {
                self.workspace.status_line = "turn interrupted".to_string();
            }
            Pending::Command(label) => {
                self.workspace.status_line = format!("/{label} completed");
            }
            Pending::Skills { announce } => self.show_skills(result, announce),
            Pending::Permissions {
                target_id,
                requested,
            } => self.show_permissions(result, &target_id, requested.as_deref())?,
            Pending::PermissionUpdate => {}
        }
        Ok(())
    }

    fn request_list(&mut self, all_workspaces: bool, cursor: Option<&str>) -> Result<()> {
        if cursor.is_none() {
            self.list_generation = self.list_generation.wrapping_add(1);
            self.list_chain = Some(ListChain {
                generation: self.list_generation,
                all_workspaces,
                pages: 0,
                cursors: HashSet::new(),
                ids: HashSet::new(),
                threads: Vec::new(),
            });
        }
        let generation = self.list_generation;
        let id = self.backend.request(
            "thread/list",
            list_params((!all_workspaces).then_some(self.cwd.as_path()), cursor),
        )?;
        self.pending.insert(id, Pending::List { generation });
        self.last_refresh = Instant::now();
        Ok(())
    }

    fn apply_list(&mut self, result: &Value, generation: u64) -> Result<()> {
        let Some(chain) = self.list_chain.as_mut() else {
            return Ok(());
        };
        if chain.generation != generation {
            return Ok(());
        }
        chain.pages += 1;
        if chain.pages > MAX_LIST_PAGES {
            self.list_chain = None;
            self.workspace.status_line =
                "session list stopped: pagination exceeded 100 pages".to_string();
            return Ok(());
        }
        for thread in result
            .get("data")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(id) = thread.get("id").and_then(Value::as_str) else {
                continue;
            };
            if chain.ids.insert(id.to_string()) {
                chain.threads.push(thread.clone());
            }
            if chain.threads.len() > MAX_LIST_ITEMS {
                self.list_chain = None;
                self.workspace.status_line =
                    "session list stopped: pagination exceeded 20000 unique threads".to_string();
                return Ok(());
            }
        }
        if let Some(cursor) = result.get("nextCursor").and_then(Value::as_str) {
            let all_workspaces = chain.all_workspaces;
            if !chain.cursors.insert(cursor.to_string()) {
                self.list_chain = None;
                self.workspace.status_line =
                    "session list stopped: backend repeated a pagination cursor".to_string();
                return Ok(());
            }
            return self.request_list(all_workspaces, Some(cursor));
        }
        let Some(chain) = self.list_chain.take() else {
            return Ok(());
        };
        let all_workspaces = chain.all_workspaces;
        for thread in &chain.threads {
            self.upsert_thread(thread);
        }
        if !self.session_decided {
            let merged = json!({"data": chain.threads});
            self.workspace
                .show_session_picker(candidates_from_list(&merged));
            let scope = if all_workspaces {
                "all workspaces"
            } else {
                "this workspace"
            };
            let storage = if self.codex_home_was_empty {
                " · warning: storage was new or empty at startup"
            } else {
                ""
            };
            self.workspace.status_line = format!(
                "choose a session from {scope} · CODEX_HOME={}{}",
                self.codex_home.display(),
                storage
            );
            return Ok(());
        }
        self.workspace.rebuild_tree(self.preferred_root.as_deref());
        if self.workspace.root_id.is_none() {
            let id = self
                .backend
                .request("thread/start", json!({"cwd": self.cwd.to_string_lossy()}))?;
            self.pending.insert(id, Pending::Start);
            self.workspace.status_line = "creating Main thread…".to_string();
            return Ok(());
        }
        self.request_descendants(None)?;
        self.request_unloaded_history()
    }

    fn select_session(&mut self, selection: Option<SessionSelection>) -> Result<()> {
        self.session_decided = true;
        if let Some(selection) = selection {
            let root_id = selection.id;
            let candidate = if selection.use_saved_cwd {
                PathBuf::from(&selection.saved_cwd)
            } else {
                self.cwd.clone()
            };
            let resume_cwd = match validated_resume_cwd(&candidate) {
                Ok(cwd) => cwd,
                Err(error) => {
                    self.session_decided = false;
                    self.workspace.status_line = if selection.use_saved_cwd {
                        format!(
                            "saved workspace {error}; choose c only from a safe current directory"
                        )
                    } else {
                        format!("current workspace {error}; restart from a safe existing directory")
                    };
                    return Ok(());
                }
            };
            self.workspace.clear_session_picker();
            self.starting_new_session = false;
            self.preferred_root = Some(root_id.clone());
            self.workspace.status_line = format!("opening {root_id}…");
            let id = self
                .backend
                .request("thread/resume", thread_resume_params(&root_id, &resume_cwd))?;
            self.pending.insert(
                id,
                Pending::OpenThread {
                    thread_id: root_id,
                    resume_cwd,
                },
            );
            Ok(())
        } else {
            self.starting_new_session = true;
            self.workspace.show_session_starting();
            self.preferred_root = None;
            self.loaded_history.clear();
            self.workspace.threads.clear();
            self.workspace.order.clear();
            self.workspace.root_id = None;
            self.workspace.selected = 0;
            let id = self
                .backend
                .request("thread/start", json!({"cwd": self.cwd.to_string_lossy()}))?;
            self.pending.insert(id, Pending::Start);
            self.workspace.status_line = "creating a new Main thread…".to_string();
            Ok(())
        }
    }

    fn choose_session(&mut self) -> Result<()> {
        self.session_decided = false;
        self.starting_new_session = false;
        self.workspace.status_line = format!(
            "loading all saved sessions · CODEX_HOME={}",
            self.codex_home.display()
        );
        self.request_list(true, None)
    }

    fn request_descendants(&mut self, cursor: Option<&str>) -> Result<()> {
        let Some(root_id) = self.workspace.root_id.clone() else {
            return Ok(());
        };
        if cursor.is_none() {
            self.descendants_generation = self.descendants_generation.wrapping_add(1);
            self.descendants_chain = Some(DescendantChain {
                generation: self.descendants_generation,
                root_id: root_id.clone(),
                pages: 0,
                cursors: HashSet::new(),
                ids: HashSet::new(),
            });
        }
        let generation = self.descendants_generation;
        self.request_descendants_page(&root_id, generation, cursor)
    }

    fn request_descendants_page(
        &mut self,
        root_id: &str,
        generation: u64,
        cursor: Option<&str>,
    ) -> Result<()> {
        let id = self.backend.request(
            "thread/list",
            json!({
                "limit": 200,
                "sortKey": "created_at",
                "sortDirection": "asc",
                "sourceKinds": [
                    "subAgent",
                    "subAgentReview",
                    "subAgentCompact",
                    "subAgentThreadSpawn",
                    "subAgentOther"
                ],
                "archived": false,
                "ancestorThreadId": root_id,
                "cursor": cursor
            }),
        )?;
        self.pending.insert(
            id,
            Pending::Descendants {
                root_id: root_id.to_string(),
                generation,
            },
        );
        Ok(())
    }

    fn apply_descendants(&mut self, root_id: &str, generation: u64, result: &Value) -> Result<()> {
        let Some(chain) = self.descendants_chain.as_mut() else {
            return Ok(());
        };
        if chain.generation != generation || chain.root_id != root_id {
            return Ok(());
        }
        chain.pages += 1;
        if chain.pages > MAX_LIST_PAGES {
            self.descendants_chain = None;
            self.workspace.status_line =
                "sub-agent list stopped: pagination exceeded 100 pages".to_string();
            return Ok(());
        }
        let mut unique = Vec::new();
        for thread in result
            .get("data")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(id) = thread.get("id").and_then(Value::as_str) else {
                continue;
            };
            if chain.ids.insert(id.to_string()) {
                unique.push(thread.clone());
            }
            if chain.ids.len() > MAX_LIST_ITEMS {
                self.descendants_chain = None;
                self.workspace.status_line =
                    "sub-agent list stopped: pagination exceeded 20000 unique threads".to_string();
                return Ok(());
            }
        }
        for thread in &unique {
            self.upsert_thread(thread);
        }
        self.workspace.rebuild_tree(Some(root_id));
        if let Some(cursor) = result.get("nextCursor").and_then(Value::as_str) {
            let chain = self
                .descendants_chain
                .as_mut()
                .expect("active descendant chain");
            if !chain.cursors.insert(cursor.to_string()) {
                self.descendants_chain = None;
                self.workspace.status_line =
                    "sub-agent list stopped: backend repeated a pagination cursor".to_string();
                return Ok(());
            }
            return self.request_descendants_page(root_id, generation, Some(cursor));
        }
        self.descendants_chain = None;
        self.request_unloaded_history()
    }

    fn request_unloaded_history(&mut self) -> Result<()> {
        let ids = self.workspace.order.clone();
        for thread_id in ids {
            if self.loaded_history.contains(&thread_id) {
                continue;
            }
            if self
                .pending
                .values()
                .any(|pending| matches!(pending, Pending::Read(id) if id == &thread_id))
            {
                continue;
            }
            let id = self.backend.request(
                "thread/read",
                json!({"threadId": thread_id, "includeTurns": true}),
            )?;
            self.pending.insert(id, Pending::Read(thread_id));
        }
        Ok(())
    }

    fn upsert_thread(&mut self, value: &Value) {
        let Some(thread) = AgentThread::from_json(value) else {
            return;
        };
        if let Some(existing) = self.workspace.threads.get_mut(&thread.id) {
            existing.merge_metadata(value);
        } else {
            self.workspace.threads.insert(thread.id.clone(), thread);
        }
    }

    fn read_selected(&mut self) -> Result<()> {
        let Some(thread_id) = self.workspace.selected_id().map(ToOwned::to_owned) else {
            return Ok(());
        };
        if self.loaded_history.contains(&thread_id)
            || self
                .pending
                .values()
                .any(|pending| matches!(pending, Pending::Read(id) if id == &thread_id))
        {
            return Ok(());
        }
        let id = self.backend.request(
            "thread/read",
            json!({"threadId": thread_id, "includeTurns": true}),
        )?;
        self.pending.insert(id, Pending::Read(thread_id));
        Ok(())
    }

    fn submit(&mut self, submission: Submission) -> Result<()> {
        if submission.input.len() == 1
            && let Some(SubmissionInput::Text(text)) = submission.input.first()
            && let Some(command) = command::parse(text)
        {
            return self.run_slash_command(command.name, command.args);
        }
        let Some(selected) = self.workspace.selected_thread() else {
            return Ok(());
        };
        let selected_id = selected.id.clone();
        let selected_label = selected.label.clone();
        let direct = selected.can_accept_direct_input;
        let is_main = selected.parent_id.is_none();
        if !direct && !is_main {
            self.workspace.status_line = format!(
                "{selected_label} is owned by Main and cannot receive private direct input; start a new direct sub-agent with Ctrl-A"
            );
            if let Some(thread) = self.workspace.threads.get_mut(&selected_id) {
                thread.push_activity_message(
                    "Message not sent: this older parent-owned agent would copy the exchange into Main. Start a direct sub-agent with Ctrl-A instead."
                        .to_string(),
                );
            }
            return Ok(());
        }
        if let Some(thread) = self.workspace.threads.get_mut(&selected_id) {
            thread.push_user_message(submission.displayed_text.clone());
        }
        let target_id = selected_id;
        let can_send = self
            .workspace
            .threads
            .get(&target_id)
            .is_some_and(|thread| thread.can_accept_direct_input);
        if can_send {
            self.start_turn(&target_id, &submission.input)
        } else {
            let resume_cwd = match validated_resume_cwd(&self.active_session_cwd) {
                Ok(cwd) => cwd,
                Err(error) => {
                    self.workspace.status_line =
                        format!("cannot resume {target_id}: workspace {error}");
                    return Ok(());
                }
            };
            let id = self.backend.request(
                "thread/resume",
                thread_resume_params(&target_id, &resume_cwd),
            )?;
            self.pending.insert(
                id,
                Pending::ResumeAndSend {
                    target_id,
                    submission,
                },
            );
            Ok(())
        }
    }

    fn run_slash_command(&mut self, name: &str, args: &str) -> Result<()> {
        let Some(thread_id) = self.workspace.selected_id().map(ToOwned::to_owned) else {
            return Ok(());
        };
        let (method, params) = match name {
            "new" | "clear" => return self.select_session(None),
            "resume" => return self.choose_session(),
            "skills" => {
                self.request_skills(true, true)?;
                self.workspace.status_line = "loading skills…".to_string();
                return Ok(());
            }
            "status" => {
                if let Some(thread) = self.workspace.selected_thread() {
                    self.workspace.status_line = format!(
                        "{} · {} · tokens {} ({} in / {} out)",
                        thread.label,
                        thread.status,
                        thread.tokens.total,
                        thread.tokens.input,
                        thread.tokens.output
                    );
                }
                return Ok(());
            }
            "permissions" => {
                let id = self.backend.request(
                    "permissionProfile/list",
                    json!({"cwd": self.cwd.to_string_lossy(), "cursor": null, "limit": null}),
                )?;
                self.pending.insert(
                    id,
                    Pending::Permissions {
                        target_id: thread_id,
                        requested: (!args.is_empty()).then(|| args.to_string()),
                    },
                );
                self.workspace.status_line = "loading permission profiles…".to_string();
                return Ok(());
            }
            "agent" | "subagents" => {
                self.workspace.status_line =
                    "use Left/Right or click the agent bar; Ctrl-A selects Main".to_string();
                return Ok(());
            }
            "compact" => ("thread/compact/start", json!({"threadId": thread_id})),
            "rename" if !args.is_empty() => (
                "thread/name/set",
                json!({"threadId": thread_id, "name": args}),
            ),
            "fork" => (
                "thread/fork",
                json!({"threadId": thread_id, "lastTurnId": null, "beforeTurnId": null, "path": null}),
            ),
            "archive" => ("thread/archive", json!({"threadId": thread_id})),
            "delete" => ("thread/delete", json!({"threadId": thread_id})),
            "review" => {
                let prompt = if args.is_empty() {
                    "Review the current changes and find issues."
                } else {
                    args
                };
                return self.start_text_turn(&thread_id, prompt);
            }
            "init" => {
                return self.start_text_turn(
                    &thread_id,
                    "Create an AGENTS.md file with instructions for Codex in this repository.",
                );
            }
            "diff" => {
                return self.start_text_turn(&thread_id, "Show and explain the current git diff.");
            }
            "quit" | "exit" => {
                self.workspace.status_line = "press Ctrl-Q to quit".to_string();
                return Ok(());
            }
            "rename" => {
                self.workspace.status_line = "usage: /rename <name>".to_string();
                return Ok(());
            }
            _ => {
                self.workspace.status_line = format!(
                    "/{name} is a Codex TUI-only command and is not supported by this app-server frontend yet"
                );
                return Ok(());
            }
        };
        let id = self.backend.request(method, params)?;
        self.pending.insert(id, Pending::Command(name.to_string()));
        self.workspace.status_line = format!("running /{name}…");
        Ok(())
    }

    fn request_skills(&mut self, force_reload: bool, announce: bool) -> Result<()> {
        let id = self.backend.request(
            "skills/list",
            json!({"cwds": [self.cwd.to_string_lossy()], "forceReload": force_reload}),
        )?;
        self.pending.insert(id, Pending::Skills { announce });
        Ok(())
    }

    fn show_skills(&mut self, result: &Value, announce: bool) {
        let skills = result
            .pointer("/data/0/skills")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|skill| {
                skill
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(true)
            })
            .filter_map(|skill| {
                Some(SkillChoice {
                    name: skill.get("name")?.as_str()?.to_string(),
                    description: skill
                        .pointer("/interface/shortDescription")
                        .or_else(|| skill.get("description"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    path: skill.get("path")?.as_str()?.into(),
                })
            })
            .collect::<Vec<_>>();
        if announce {
            self.workspace.status_line = if skills.is_empty() {
                "no enabled skills found for this workspace".to_string()
            } else {
                format!(
                    "skills: {}",
                    skills
                        .iter()
                        .map(|skill| format!("${}", skill.name))
                        .collect::<Vec<_>>()
                        .join("  ")
                )
            };
        }
        self.workspace.set_skills(skills);
    }

    fn show_permissions(
        &mut self,
        result: &Value,
        target_id: &str,
        requested: Option<&str>,
    ) -> Result<()> {
        let entries = result
            .get("data")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        if let Some(requested) = requested {
            let matching = entries
                .iter()
                .find(|profile| profile.get("id").and_then(Value::as_str) == Some(requested));
            let message = match matching {
                Some(profile)
                    if profile
                        .get("allowed")
                        .and_then(Value::as_bool)
                        .unwrap_or(false) =>
                {
                    self.apply_permission_profile(target_id, requested)?;
                    return Ok(());
                }
                Some(_) => format!("Permission profile {requested} is blocked by requirements."),
                None => format!(
                    "Unknown permission profile {requested}. Run /permissions to list profiles."
                ),
            };
            self.workspace.status_line = message.clone();
            if let Some(thread) = self.workspace.threads.get_mut(target_id) {
                thread.push_activity_message(message);
            }
            self.workspace.scroll = u16::MAX;
            return Ok(());
        }
        let choices = entries
            .into_iter()
            .filter_map(|profile| {
                let id = profile.get("id")?.as_str()?;
                let allowed = profile
                    .get("allowed")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                allowed.then(|| PermissionChoice {
                    id: id.to_string(),
                    description: profile
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("permission profile")
                        .to_string(),
                })
            })
            .collect::<Vec<_>>();
        if choices.is_empty() {
            let message = "no allowed permission profiles available".to_string();
            self.workspace.status_line = message.clone();
            if let Some(thread) = self.workspace.threads.get_mut(target_id) {
                thread.push_activity_message(message);
            }
            return Ok(());
        }
        let current = self.permission_profiles.get(target_id).map(String::as_str);
        self.workspace
            .show_permission_picker(target_id.to_string(), choices, current);
        Ok(())
    }

    fn select_permission(&mut self, target_id: String, profile_id: String) -> Result<()> {
        self.apply_permission_profile(&target_id, &profile_id)
    }

    fn apply_permission_profile(&mut self, target_id: &str, profile_id: &str) -> Result<()> {
        self.permission_profiles
            .insert(target_id.to_string(), profile_id.to_string());
        self.workspace.set_permission_profile(target_id, profile_id);
        let id = self.backend.request(
            "thread/settings/update",
            permission_update_params(target_id, profile_id),
        )?;
        self.pending.insert(id, Pending::PermissionUpdate);
        let message = format!("Permission profile {profile_id} selected for this agent.");
        self.workspace.status_line = message.clone();
        if let Some(thread) = self.workspace.threads.get_mut(target_id) {
            thread.push_activity_message(message);
        }
        self.workspace.scroll = u16::MAX;
        Ok(())
    }

    fn start_turn(&mut self, thread_id: &str, input: &[SubmissionInput]) -> Result<()> {
        let id = self.backend.request(
            "turn/start",
            turn_start_params(
                thread_id,
                input,
                self.permission_profiles.get(thread_id).map(String::as_str),
            ),
        )?;
        self.pending.insert(id, Pending::Turn);
        Ok(())
    }

    fn start_text_turn(&mut self, thread_id: &str, text: &str) -> Result<()> {
        self.start_turn(thread_id, &[SubmissionInput::Text(text.to_string())])
    }

    fn resolve_prompt(&mut self, resolution: PromptResolution) -> Result<()> {
        self.backend
            .respond(resolution.request_id, resolution.result)?;
        if let Some((thread_id, turn_id)) = resolution.interrupt {
            self.interrupt(&thread_id, &turn_id)?;
        }
        Ok(())
    }

    fn interrupt_selected(&mut self) -> Result<()> {
        let Some(thread) = self.workspace.selected_thread() else {
            return Ok(());
        };
        let Some(turn_id) = thread.active_turn_id.clone() else {
            self.workspace.status_line = "selected agent has no active turn".to_string();
            return Ok(());
        };
        let thread_id = thread.id.clone();
        self.interrupt(&thread_id, &turn_id)
    }

    fn interrupt(&mut self, thread_id: &str, turn_id: &str) -> Result<()> {
        let id = self.backend.request(
            "turn/interrupt",
            json!({"threadId": thread_id, "turnId": turn_id}),
        )?;
        self.pending.insert(id, Pending::Interrupt);
        Ok(())
    }

    fn handle_notification(&mut self, method: &str, params: &Value) -> Result<()> {
        match method {
            "thread/started" => {
                if let Some(thread) = params.get("thread") {
                    self.upsert_thread(thread);
                    if self.session_decided && !self.starting_new_session {
                        self.workspace.rebuild_tree(self.preferred_root.as_deref());
                        self.request_unloaded_history()?;
                    }
                }
            }
            "thread/status/changed" => {
                if let (Some(thread_id), Some(status)) = (
                    params.get("threadId").and_then(Value::as_str),
                    params.get("status"),
                ) && let Some(thread) = self.workspace.threads.get_mut(thread_id)
                {
                    thread.set_status(model::status_name(Some(status)));
                }
            }
            "turn/started" => self.start_notified_turn(params),
            "turn/completed" => self.complete_notified_turn(params),
            "skills/changed" => self.request_skills(true, false)?,
            "serverRequest/resolved" => {
                if let Some(request_id) = params.get("requestId") {
                    self.workspace.clear_prompt(request_id);
                }
            }
            "thread/tokenUsage/updated" => {
                let total = params.pointer("/tokenUsage/total").unwrap_or(&Value::Null);
                let input = total
                    .get("inputTokens")
                    .and_then(Value::as_i64)
                    .unwrap_or_default();
                let output = total
                    .get("outputTokens")
                    .and_then(Value::as_i64)
                    .unwrap_or_default();
                let total = total
                    .get("totalTokens")
                    .and_then(Value::as_i64)
                    .unwrap_or_default();
                if let Some(thread) = self.notification_thread_mut(params) {
                    thread.tokens.input = input;
                    thread.tokens.output = output;
                    thread.tokens.total = total;
                }
            }
            "item/agentMessage/delta" => {
                let item_id = params
                    .get("itemId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let delta = params
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if let Some(thread) = self.notification_thread_mut(params) {
                    thread.append_delta(item_id, delta);
                }
            }
            "item/started" | "item/completed" => {
                let item = params.get("item").unwrap_or(&Value::Null).clone();
                let completed = method == "item/completed";
                let source_id = params
                    .get("threadId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let item_id = item.get("id").and_then(Value::as_str).unwrap_or_default();
                let item_key = (source_id.to_string(), item_id.to_string());
                if completed {
                    self.live_items.remove(&item_key);
                } else if !source_id.is_empty() && !item_id.is_empty() {
                    self.live_items.insert(item_key, item.clone());
                }
                if let Some(thread) = self.workspace.threads.get_mut(source_id) {
                    thread.update_activity(&item);
                }
                if let Some(thread) = self.notification_thread_mut(params) {
                    thread.update_item(&item, completed);
                }
            }
            "item/fileChange/patchUpdated" => {
                let thread_id = params
                    .get("threadId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let item_id = params
                    .get("itemId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if !thread_id.is_empty() && !item_id.is_empty() {
                    self.live_items.insert(
                        (thread_id.to_string(), item_id.to_string()),
                        json!({
                            "type": "fileChange",
                            "id": item_id,
                            "changes": params.get("changes").cloned().unwrap_or_else(|| json!([])),
                            "status": "inProgress"
                        }),
                    );
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn notification_thread_mut(&mut self, params: &Value) -> Option<&mut AgentThread> {
        let thread_id = params.get("threadId")?.as_str()?;
        self.workspace.threads.get_mut(thread_id)
    }

    fn start_notified_turn(&mut self, params: &Value) {
        let turn_id = params
            .pointer("/turn/id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        if let Some(thread) = self.notification_thread_mut(params) {
            if let Some(turn_id) = turn_id {
                thread.start_turn(turn_id);
            } else {
                thread.set_status("working".to_string());
            }
        }
    }

    fn complete_notified_turn(&mut self, params: &Value) {
        if let Some(thread) = self.notification_thread_mut(params) {
            thread.complete_turn();
        }
    }
}

fn list_params(cwd: Option<&std::path::Path>, cursor: Option<&str>) -> Value {
    let mut params = json!({
        "limit": 200,
        "sortKey": "updated_at",
        "sortDirection": "desc",
        "sourceKinds": SOURCE_KINDS,
        "archived": false,
        "cursor": cursor
    });
    if let Some(cwd) = cwd {
        params["cwd"] = json!(cwd.to_string_lossy());
    }
    params
}

fn thread_resume_params(thread_id: &str, cwd: &std::path::Path) -> Value {
    json!({"threadId": thread_id, "cwd": cwd.to_string_lossy()})
}

fn validated_resume_cwd(path: &std::path::Path) -> std::result::Result<PathBuf, &'static str> {
    if !path.is_dir() {
        return Err("is missing or is not a directory");
    }
    if is_trash_path(path) {
        return Err("is inside Trash");
    }
    Ok(path.to_path_buf())
}

fn is_trash_path(path: &std::path::Path) -> bool {
    let components = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>();

    components
        .windows(2)
        .any(|window| window[0] == "Trash" && window[1] == "files")
        || components.windows(3).any(|window| {
            (window[0] == ".Trash"
                && !window[1].is_empty()
                && window[1]
                    .chars()
                    .all(|character| character.is_ascii_digit())
                && window[2] == "files")
                || (window[0] == ".Trashes"
                    && !window[1].is_empty()
                    && window[1]
                        .chars()
                        .all(|character| character.is_ascii_digit()))
        })
        || components.windows(2).any(|window| {
            window[0].strip_prefix(".Trash-").is_some_and(|suffix| {
                !suffix.is_empty() && suffix.chars().all(|character| character.is_ascii_digit())
            }) && window[1] == "files"
        })
        || (components.first().is_some_and(|root| root == "/")
            && components.get(1).is_some_and(|users| users == "Users")
            && components.get(2).is_some_and(|user| !user.is_empty())
            && components.get(3).is_some_and(|trash| trash == ".Trash"))
}

fn directory_is_empty_or_missing(path: &std::path::Path) -> bool {
    std::fs::read_dir(path)
        .map(|mut entries| entries.next().is_none())
        .unwrap_or(true)
}

fn permission_update_params(thread_id: &str, profile_id: &str) -> Value {
    json!({
        "threadId": thread_id,
        "permissions": profile_id
    })
}

fn turn_start_params(
    thread_id: &str,
    input: &[SubmissionInput],
    permissions: Option<&str>,
) -> Value {
    let input = input
        .iter()
        .map(|item| match item {
            SubmissionInput::Text(text) => {
                json!({"type": "text", "text": text, "textElements": []})
            }
            SubmissionInput::LocalImage(path) => {
                json!({"type": "localImage", "path": path})
            }
            SubmissionInput::Skill { name, path } => {
                json!({"type": "skill", "name": name, "path": path})
            }
        })
        .collect::<Vec<_>>();
    json!({
        "threadId": thread_id,
        "input": input,
        "permissions": permissions
    })
}
