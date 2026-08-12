mod backend;
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
use crossterm::event::DisableMouseCapture;
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
    List,
    Descendants(String),
    Read(String),
    Start,
    ResumeAndSend {
        target_id: String,
        text: String,
    },
    Turn,
    Interrupt,
    Command(String),
    Skills,
    Permissions {
        target_id: String,
        requested: Option<String>,
    },
    PermissionUpdate,
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
    last_refresh: Instant,
    permission_profiles: HashMap<String, String>,
    codex: PathBuf,
    codex_home: PathBuf,
    update_result: Option<Receiver<std::result::Result<String, String>>>,
}

fn main() -> Result<()> {
    let (wrapper_args, codex_args) = split_args(env::args_os());
    let args = Args::parse_from(wrapper_args);
    if args.combined_help {
        print_combined_help(&args.codex)?;
        return Ok(());
    }
    let codex_home = args.codex_home.unwrap_or_else(default_codex_home);
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
        cwd,
        last_refresh: Instant::now(),
        permission_profiles: HashMap::new(),
        codex: args.codex,
        codex_home,
        update_result: None,
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

fn print_combined_help(codex: &std::path::Path) -> Result<()> {
    use clap::CommandFactory;

    Args::command().print_help()?;
    println!("\n\nInstalled Codex options ({}):\n", codex.display());
    let status = Command::new(codex)
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
                let list_id = backend.request("thread/list", list_params(cwd))?;
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
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;
    let result = run_event_loop(app, &mut terminal);
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;
    result
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
        if app.session_decided
            && !app.starting_new_session
            && app.last_refresh.elapsed() >= Duration::from_secs(2)
            && app
                .pending
                .values()
                .all(|pending| !matches!(pending, Pending::List))
        {
            app.request_list()?;
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
            Action::Submit(text) => app.submit(text)?,
            Action::SelectionChanged => app.read_selected()?,
            Action::ResolvePrompt(resolution) => app.resolve_prompt(resolution)?,
            Action::Interrupt => app.interrupt_selected()?,
            Action::SessionSelected(root_id) => app.select_session(root_id)?,
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
    if let Err(error) = execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    ) {
        let _ = enable_raw_mode();
        return Err(error.into());
    }
    if let Err(error) = terminal.show_cursor() {
        let _ = resume_terminal(terminal);
        return Err(error.into());
    }
    Ok(())
}

fn resume_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    enable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        EnterAlternateScreen,
        EnableMouseCapture
    )?;
    terminal.clear()?;
    Ok(())
}

impl App {
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
                self.request_list()?;
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
                self.request_list()?;
            }
            Pending::List => self.apply_list(result)?,
            Pending::Descendants(root_id) => self.apply_descendants(&root_id, result)?,
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
            Pending::ResumeAndSend { target_id, text } => self.start_turn(&target_id, &text)?,
            Pending::Turn => {}
            Pending::Interrupt => {
                self.workspace.status_line = "turn interrupted".to_string();
            }
            Pending::Command(label) => {
                self.workspace.status_line = format!("/{label} completed");
            }
            Pending::Skills => self.show_skills(result),
            Pending::Permissions {
                target_id,
                requested,
            } => self.show_permissions(result, &target_id, requested.as_deref())?,
            Pending::PermissionUpdate => {}
        }
        Ok(())
    }

    fn request_list(&mut self) -> Result<()> {
        let id = self
            .backend
            .request("thread/list", list_params(&self.cwd))?;
        self.pending.insert(id, Pending::List);
        self.last_refresh = Instant::now();
        Ok(())
    }

    fn apply_list(&mut self, result: &Value) -> Result<()> {
        for thread in result
            .get("data")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            self.upsert_thread(thread);
        }
        if !self.session_decided {
            self.workspace
                .show_session_picker(candidates_from_list(result));
            self.workspace.status_line = "choose a session".to_string();
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
        self.request_descendants()?;
        self.request_unloaded_history()
    }

    fn select_session(&mut self, root_id: Option<String>) -> Result<()> {
        self.session_decided = true;
        if let Some(root_id) = root_id {
            self.workspace.clear_session_picker();
            self.starting_new_session = false;
            self.preferred_root = Some(root_id.clone());
            self.workspace.rebuild_tree(Some(&root_id));
            self.workspace.status_line = format!("opening {root_id}…");
            self.request_descendants()?;
            self.request_unloaded_history()
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
        self.workspace.status_line = "loading saved sessions…".to_string();
        self.request_list()
    }

    fn request_descendants(&mut self) -> Result<()> {
        let Some(root_id) = self.workspace.root_id.clone() else {
            return Ok(());
        };
        if self
            .pending
            .values()
            .any(|pending| matches!(pending, Pending::Descendants(id) if id == &root_id))
        {
            return Ok(());
        }
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
                "ancestorThreadId": root_id
            }),
        )?;
        self.pending.insert(id, Pending::Descendants(root_id));
        Ok(())
    }

    fn apply_descendants(&mut self, root_id: &str, result: &Value) -> Result<()> {
        for thread in result
            .get("data")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            self.upsert_thread(thread);
        }
        self.workspace.rebuild_tree(Some(root_id));
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

    fn submit(&mut self, text: String) -> Result<()> {
        if let Some(command) = command::parse(&text) {
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
            thread.push_user_message(text.clone());
        }
        let target_id = selected_id;
        let can_send = self
            .workspace
            .threads
            .get(&target_id)
            .is_some_and(|thread| thread.can_accept_direct_input);
        if can_send {
            self.start_turn(&target_id, &text)
        } else {
            let id = self
                .backend
                .request("thread/resume", json!({"threadId": target_id}))?;
            self.pending
                .insert(id, Pending::ResumeAndSend { target_id, text });
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
                let id = self.backend.request(
                    "skills/list",
                    json!({"cwds": [self.cwd.to_string_lossy()], "forceReload": true}),
                )?;
                self.pending.insert(id, Pending::Skills);
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
                return self.start_turn(&thread_id, prompt);
            }
            "init" => {
                return self.start_turn(
                    &thread_id,
                    "Create an AGENTS.md file with instructions for Codex in this repository.",
                );
            }
            "diff" => return self.start_turn(&thread_id, "Show and explain the current git diff."),
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

    fn show_skills(&mut self, result: &Value) {
        let names = result
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
            .filter_map(|skill| skill.get("name").and_then(Value::as_str))
            .map(|name| format!("${name}"))
            .collect::<Vec<_>>();
        self.workspace.status_line = if names.is_empty() {
            "no enabled skills found for this workspace".to_string()
        } else {
            format!("skills: {}", names.join("  "))
        };
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

    fn start_turn(&mut self, thread_id: &str, text: &str) -> Result<()> {
        let id = self.backend.request(
            "turn/start",
            turn_start_params(
                thread_id,
                text,
                self.permission_profiles.get(thread_id).map(String::as_str),
            ),
        )?;
        self.pending.insert(id, Pending::Turn);
        Ok(())
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

fn list_params(cwd: &std::path::Path) -> Value {
    json!({
        "limit": 200,
        "sortKey": "updated_at",
        "sortDirection": "desc",
        "sourceKinds": SOURCE_KINDS,
        "archived": false,
        "cwd": cwd.to_string_lossy()
    })
}

fn permission_update_params(thread_id: &str, profile_id: &str) -> Value {
    json!({
        "threadId": thread_id,
        "permissions": profile_id
    })
}

fn turn_start_params(thread_id: &str, text: &str, permissions: Option<&str>) -> Value {
    json!({
        "threadId": thread_id,
        "input": [{"type": "text", "text": text, "textElements": []}],
        "permissions": permissions
    })
}
