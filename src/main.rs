mod backend;
mod model;
mod prompt;
mod ui;

use std::collections::HashMap;
use std::collections::HashSet;
use std::env;
use std::io;
use std::path::PathBuf;
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
use model::AgentThread;
use prompt::PromptResolution;
use prompt::ServerPrompt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use serde_json::Value;
use serde_json::json;
use ui::Action;
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
#[command(version, about)]
struct Args {
    /// Installed Codex executable to use as the backend.
    #[arg(long, env = "CODEX_BIN", default_value = "codex")]
    codex: PathBuf,

    /// Persistent test home used by the installed Codex backend.
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
}

#[derive(Debug)]
enum Pending {
    Initialize,
    List,
    Descendants(String),
    Read(String),
    Start,
    ResumeAndSend { target_id: String, text: String },
    Turn,
    Interrupt,
}

struct App {
    backend: Backend,
    workspace: Workspace,
    pending: HashMap<u64, Pending>,
    loaded_history: HashSet<String>,
    preferred_root: Option<String>,
    cwd: PathBuf,
    last_refresh: Instant,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let codex_home = args.codex_home.unwrap_or_else(default_codex_home);
    let cwd = args
        .cwd
        .unwrap_or(env::current_dir().context("failed to read cwd")?);
    let mut backend = Backend::spawn(&args.codex, &codex_home)?;
    if args.check_backend {
        return check_backend(&mut backend, &args.codex, &codex_home, &cwd);
    }
    let initialize_id = backend.initialize()?;
    let mut app = App {
        backend,
        workspace: Workspace::new(),
        pending: HashMap::from([(initialize_id, Pending::Initialize)]),
        loaded_history: HashSet::new(),
        preferred_root: args.thread,
        cwd,
        last_refresh: Instant::now(),
    };
    run_terminal(&mut app)
}

fn default_codex_home() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("tmp/codex-agent-picker-test-home")
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
        if app.last_refresh.elapsed() >= Duration::from_secs(2)
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
            Event::Resize(_, _) | Event::FocusGained | Event::FocusLost | Event::Paste(_) => {
                Action::None
            }
        };
        match action {
            Action::Quit => return Ok(()),
            Action::Submit(text) => app.submit(text)?,
            Action::SelectionChanged => app.read_selected()?,
            Action::ResolvePrompt(resolution) => app.resolve_prompt(resolution)?,
            Action::Interrupt => app.interrupt_selected()?,
            Action::None => {}
        }
    }
}

impl App {
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
            match ServerPrompt::from_request(&message) {
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
                        thread.log.push(format!("Action required: {method}"));
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
            self.workspace.status_line = format!("app-server error: {error}");
            return Ok(());
        }
        let result = message.get("result").unwrap_or(&Value::Null);
        match pending {
            Pending::Initialize => {
                self.backend.initialized()?;
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
        let id = self.backend.request(
            "thread/read",
            json!({"threadId": thread_id, "includeTurns": true}),
        )?;
        self.pending.insert(id, Pending::Read(thread_id));
        Ok(())
    }

    fn submit(&mut self, text: String) -> Result<()> {
        let Some(selected) = self.workspace.selected_thread() else {
            return Ok(());
        };
        let selected_id = selected.id.clone();
        let selected_label = selected.label.clone();
        let direct = selected.can_accept_direct_input;
        let is_main = selected.parent_id.is_none();
        if let Some(thread) = self.workspace.threads.get_mut(&selected_id) {
            thread.log.push(format!("You: {text}"));
        }
        let (target_id, routed_text) = if direct || is_main {
            (selected_id, text)
        } else {
            let Some(root_id) = self.workspace.root_id.clone() else {
                return Ok(());
            };
            (
                root_id,
                format!(
                    "Пользователь выбрал субагента {selected_label} ({selected_id}). Передай ему это сообщение через средство связи с субагентом и покажи его ответ в его потоке:\n\n{text}"
                ),
            )
        };
        let can_send = self
            .workspace
            .threads
            .get(&target_id)
            .is_some_and(|thread| thread.can_accept_direct_input);
        if can_send {
            self.start_turn(&target_id, &routed_text)
        } else {
            let id = self
                .backend
                .request("thread/resume", json!({"threadId": target_id}))?;
            self.pending.insert(
                id,
                Pending::ResumeAndSend {
                    target_id,
                    text: routed_text,
                },
            );
            Ok(())
        }
    }

    fn start_turn(&mut self, thread_id: &str, text: &str) -> Result<()> {
        let id = self.backend.request(
            "turn/start",
            json!({
                "threadId": thread_id,
                "input": [{"type": "text", "text": text, "textElements": []}]
            }),
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
                    self.workspace.rebuild_tree(self.preferred_root.as_deref());
                    self.request_unloaded_history()?;
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
                if let Some(thread) = self.notification_thread_mut(params) {
                    thread.complete_item(&item);
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
