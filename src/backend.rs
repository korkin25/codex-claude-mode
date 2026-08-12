use std::ffi::OsString;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Write;
use std::path::Path;
use std::process::Child;
use std::process::ChildStdin;
use std::process::Command;
use std::process::Stdio;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

use anyhow::Context;
use anyhow::Result;
use serde_json::Value;
use serde_json::json;

pub(crate) enum BackendEvent {
    Message(Value),
    Stderr(String),
    Exited,
}

pub(crate) struct Backend {
    child: Child,
    stdin: ChildStdin,
    events: Receiver<BackendEvent>,
    next_request_id: u64,
}

impl Backend {
    pub(crate) fn spawn(codex: &Path, codex_home: &Path, codex_args: &[OsString]) -> Result<Self> {
        std::fs::create_dir_all(codex_home)
            .with_context(|| format!("failed to create CODEX_HOME {}", codex_home.display()))?;
        let mut child = Command::new(codex)
            .args(codex_args)
            .arg("-c")
            .arg("features.multi_agent_v2=false")
            .arg("app-server")
            .env("CODEX_HOME", codex_home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to start {} app-server", codex.display()))?;
        let stdin = child
            .stdin
            .take()
            .context("app-server stdin is unavailable")?;
        let stdout = child
            .stdout
            .take()
            .context("app-server stdout is unavailable")?;
        let stderr = child
            .stderr
            .take()
            .context("app-server stderr is unavailable")?;
        let (event_tx, events) = mpsc::channel();
        spawn_stdout_reader(stdout, event_tx.clone());
        spawn_stderr_reader(stderr, event_tx);
        Ok(Self {
            child,
            stdin,
            events,
            next_request_id: 1,
        })
    }

    pub(crate) fn initialize(&mut self) -> Result<u64> {
        self.request(
            "initialize",
            json!({
                "clientInfo": {
                    "name": "codex-claude-mode",
                    "title": "Codex Agent Workspace",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "capabilities": {
                    "experimentalApi": true,
                    "requestAttestation": false,
                    "mcpServerOpenaiFormElicitation": false
                }
            }),
        )
    }

    pub(crate) fn initialized(&mut self) -> Result<()> {
        self.notification("initialized", Value::Null)
    }

    pub(crate) fn request(&mut self, method: &str, params: Value) -> Result<u64> {
        let id = self.next_request_id;
        self.next_request_id += 1;
        self.write(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        Ok(id)
    }

    fn notification(&mut self, method: &str, params: Value) -> Result<()> {
        self.write(json!({"jsonrpc": "2.0", "method": method, "params": params}))
    }

    fn write(&mut self, message: Value) -> Result<()> {
        serde_json::to_writer(&mut self.stdin, &message).context("failed to encode request")?;
        self.stdin
            .write_all(b"\n")
            .context("failed to write request")?;
        self.stdin.flush().context("failed to flush request")
    }

    pub(crate) fn try_recv(&self) -> Option<BackendEvent> {
        self.events.try_recv().ok()
    }

    pub(crate) fn recv_timeout(&self, timeout: Duration) -> Option<BackendEvent> {
        self.events.recv_timeout(timeout).ok()
    }

    pub(crate) fn reject_server_request(&mut self, id: &Value, method: &str) -> Result<()> {
        self.write(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32601,
                "message": format!("codex-claude-mode does not yet handle {method}")
            }
        }))
    }

    pub(crate) fn respond(&mut self, id: Value, result: Value) -> Result<()> {
        self.write(json!({"jsonrpc": "2.0", "id": id, "result": result}))
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn_stdout_reader(
    stdout: impl std::io::Read + Send + 'static,
    event_tx: Sender<BackendEvent>,
) {
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            match line {
                Ok(line) => match serde_json::from_str(&line) {
                    Ok(message) => {
                        if event_tx.send(BackendEvent::Message(message)).is_err() {
                            return;
                        }
                    }
                    Err(error) => {
                        let _ = event_tx.send(BackendEvent::Stderr(format!(
                            "invalid app-server response: {error}"
                        )));
                    }
                },
                Err(error) => {
                    let _ = event_tx.send(BackendEvent::Stderr(error.to_string()));
                    break;
                }
            }
        }
        let _ = event_tx.send(BackendEvent::Exited);
    });
}

fn spawn_stderr_reader(
    stderr: impl std::io::Read + Send + 'static,
    event_tx: Sender<BackendEvent>,
) {
    thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if event_tx.send(BackendEvent::Stderr(line)).is_err() {
                return;
            }
        }
    });
}
