use std::fs;
use std::fs::OpenOptions;
use std::hash::Hash;
use std::hash::Hasher;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;

use anyhow::Context;
use anyhow::Result;
use anyhow::bail;

const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 1024 * 1024;
const MAX_DIAGNOSTIC_BYTES: u64 = 4 * 1024;
const CLIPBOARD_TIMEOUT: Duration = Duration::from_secs(3);
#[cfg(unix)]
const PROCESS_GROUP_TERM_GRACE: Duration = Duration::from_millis(100);
#[cfg(unix)]
const MAX_READS_PER_POLL: usize = 8;
const STALE_AFTER: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const DIRECTORY_PREFIX: &str = "session-";
const ROOT_DIRECTORY: &str = "codex-claude-mode-clipboard";

pub(crate) struct ClipboardImage {
    pub(crate) path: PathBuf,
    pub(crate) format: &'static str,
    pub(crate) size: usize,
}

pub(crate) struct CapturedImage {
    bytes: Vec<u8>,
    format: &'static str,
    extension: &'static str,
}

pub(crate) enum CapturedClipboard {
    Image(CapturedImage),
    Text(String),
}

pub(crate) struct ClipboardImages {
    root: PathBuf,
    directory: PathBuf,
    next_number: usize,
}

pub(crate) struct ClipboardCapture {
    receiver: Receiver<Result<CapturedClipboard>>,
    cancel: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl ClipboardCapture {
    pub(crate) fn try_recv(&self) -> Result<Result<CapturedClipboard>, mpsc::TryRecvError> {
        self.receiver.try_recv()
    }
}

impl Drop for ClipboardCapture {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl ClipboardImages {
    pub(crate) fn new() -> Self {
        let root = clipboard_root(&std::env::temp_dir());
        let now = SystemTime::now();
        if ensure_private_root(&root).is_ok() {
            Self::new_in(root, now)
        } else {
            Self::from_root(root, now)
        }
    }

    fn new_in(root: PathBuf, now: SystemTime) -> Self {
        cleanup_stale_directories(&root, now);
        Self::from_root(root, now)
    }

    fn from_root(root: PathBuf, now: SystemTime) -> Self {
        let nonce = now
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let directory = root.join(format!("{DIRECTORY_PREFIX}{}-{nonce}", std::process::id()));
        Self {
            root,
            directory,
            next_number: 1,
        }
    }

    pub(crate) fn capture_in_background() -> ClipboardCapture {
        let (sender, receiver) = mpsc::sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let worker = thread::spawn(move || {
            let runner = SystemRunner {
                cancel: &worker_cancel,
            };
            let result = capture_clipboard(&runner, current_backend());
            let _ = sender.send(result);
        });
        ClipboardCapture {
            receiver,
            cancel,
            worker: Some(worker),
        }
    }

    pub(crate) fn store(&mut self, captured: CapturedImage) -> Result<ClipboardImage> {
        ensure_private_root(&self.root)?;
        if !self.directory.exists() {
            create_private_directory(&self.directory)?;
        }
        let path = self
            .directory
            .join(format!("image-{}.{}", self.next_number, captured.extension));
        self.next_number += 1;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        set_private_file_options(&mut options);
        let mut file = options
            .open(&path)
            .with_context(|| format!("failed to create {}", path.display()))?;
        std::io::Write::write_all(&mut file, &captured.bytes)?;
        Ok(ClipboardImage {
            path,
            format: captured.format,
            size: captured.bytes.len(),
        })
    }
}

fn decode_text(bytes: Vec<u8>, limit: usize) -> Result<String> {
    if bytes.len() > limit {
        bail!("clipboard text exceeds configured limit")
    }
    String::from_utf8(bytes).context("clipboard text is not UTF-8")
}

fn clipboard_root(temp: &Path) -> PathBuf {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    user_identity().hash(&mut hasher);
    temp.join(format!("{ROOT_DIRECTORY}-{:016x}", hasher.finish()))
}

#[cfg(unix)]
fn user_identity() -> String {
    use std::os::unix::fs::MetadataExt;
    std::env::var_os("HOME")
        .and_then(|home| fs::metadata(home).ok())
        .map(|metadata| format!("uid:{}", metadata.uid()))
        .unwrap_or_else(|| format!("process:{}", std::process::id()))
}

#[cfg(not(unix))]
fn user_identity() -> String {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map_or_else(
            || format!("process:{}", std::process::id()),
            |home| home.to_string_lossy().into_owned(),
        )
}

fn ensure_private_root(path: &Path) -> Result<()> {
    match create_private_directory(path) {
        Ok(()) => Ok(()),
        Err(_error) if path.is_dir() && private_directory_permissions(path) => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
fn private_directory_permissions(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::symlink_metadata(path).is_ok_and(|metadata| {
        metadata.file_type().is_dir() && metadata.permissions().mode() & 0o077 == 0
    })
}

#[cfg(not(unix))]
fn private_directory_permissions(path: &Path) -> bool {
    path.is_dir()
}

fn cleanup_stale_directories(root: &Path, now: SystemTime) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let modified = entry.metadata().and_then(|metadata| metadata.modified());
        if name.starts_with(DIRECTORY_PREFIX)
            && file_type.is_dir()
            && modified
                .ok()
                .and_then(|modified| now.duration_since(modified).ok())
                .is_some_and(|age| age >= STALE_AFTER)
        {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

impl Drop for ClipboardImages {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

enum ImageCapture {
    Bytes(Vec<u8>),
    NoImage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Backend {
    Wayland,
    X11,
    MacOs,
}

fn current_backend() -> Backend {
    if cfg!(target_os = "macos") {
        Backend::MacOs
    } else if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        Backend::Wayland
    } else {
        Backend::X11
    }
}

trait Runner {
    fn run(&self, program: &str, args: &[&str], max_bytes: u64) -> Result<Vec<u8>, RunError>;
}

struct SystemRunner<'a> {
    cancel: &'a AtomicBool,
}

impl Runner for SystemRunner<'_> {
    fn run(&self, program: &str, args: &[&str], max_bytes: u64) -> Result<Vec<u8>, RunError> {
        run_bounded_with_limit(program, args, self.cancel, max_bytes)
    }
}

fn capture_clipboard(runner: &impl Runner, backend: Backend) -> Result<CapturedClipboard> {
    match capture_image(runner, backend)? {
        ImageCapture::Bytes(bytes) => {
            let (format, extension) = detect_image(&bytes)?;
            Ok(CapturedClipboard::Image(CapturedImage {
                bytes,
                format,
                extension,
            }))
        }
        ImageCapture::NoImage => capture_text(runner, backend).map(CapturedClipboard::Text),
    }
}

fn capture_image(runner: &impl Runner, backend: Backend) -> Result<ImageCapture> {
    match backend {
        Backend::Wayland => {
            let types = run_command(runner, "wl-paste", &["--list-types"], MAX_TEXT_BYTES as u64)?;
            let types = String::from_utf8_lossy(&types);
            let Some(mime) = preferred_image_mime(&types) else {
                return Ok(ImageCapture::NoImage);
            };
            run_command(
                runner,
                "wl-paste",
                &["--no-newline", "--type", mime],
                MAX_IMAGE_BYTES,
            )
            .map(ImageCapture::Bytes)
        }
        Backend::X11 => {
            for mime in ["image/png", "image/jpeg"] {
                match runner.run(
                    "xclip",
                    &["-selection", "clipboard", "-t", mime, "-o"],
                    MAX_IMAGE_BYTES,
                ) {
                    Ok(bytes) => return Ok(ImageCapture::Bytes(bytes)),
                    Err(error) if xclip_type_unavailable(&error) => {}
                    Err(error) => return Err(command_error("xclip image read", error)),
                }
            }
            Ok(ImageCapture::NoImage)
        }
        Backend::MacOs => match runner.run("pngpaste", &["-"], MAX_IMAGE_BYTES) {
            Ok(bytes) => Ok(ImageCapture::Bytes(bytes)),
            Err(RunError::Unavailable) => Ok(ImageCapture::NoImage),
            Err(error) if pngpaste_no_image(&error) => Ok(ImageCapture::NoImage),
            Err(error) => Err(command_error("pngpaste image read", error)),
        },
    }
}

fn capture_text(runner: &impl Runner, backend: Backend) -> Result<String> {
    let bytes = match backend {
        Backend::Wayland => {
            let mut last_unavailable = None;
            for mime in ["text/plain;charset=utf-8", "text/plain"] {
                match runner.run(
                    "wl-paste",
                    &["--no-newline", "--type", mime],
                    MAX_TEXT_BYTES as u64,
                ) {
                    Ok(bytes) => return decode_text(bytes, MAX_TEXT_BYTES),
                    Err(error) if wl_type_unavailable(&error) => last_unavailable = Some(error),
                    Err(error) => return Err(command_error("wl-paste text read", error)),
                }
            }
            return Err(command_error(
                "wl-paste text read",
                last_unavailable.unwrap_or(RunError::NoData),
            ));
        }
        Backend::X11 => run_command(
            runner,
            "xclip",
            &["-selection", "clipboard", "-o"],
            MAX_TEXT_BYTES as u64,
        )?,
        Backend::MacOs => run_command(runner, "pbpaste", &[], MAX_TEXT_BYTES as u64)?,
    };
    decode_text(bytes, MAX_TEXT_BYTES)
}

fn run_command(
    runner: &impl Runner,
    program: &str,
    args: &[&str],
    max_bytes: u64,
) -> Result<Vec<u8>> {
    runner
        .run(program, args, max_bytes)
        .map_err(|error| command_error(program, error))
}

fn command_error(operation: &str, error: RunError) -> anyhow::Error {
    anyhow::anyhow!("{operation} failed: {error}")
}

fn xclip_type_unavailable(error: &RunError) -> bool {
    matches!(error, RunError::NoData)
        || matches!(error, RunError::CommandFailed { stderr, .. } if {
            let stderr = stderr.to_ascii_lowercase();
            stderr.contains("target") && stderr.contains("not available")
        })
}

fn pngpaste_no_image(error: &RunError) -> bool {
    matches!(error, RunError::NoData)
        || matches!(error, RunError::CommandFailed { stderr, .. } if {
            let stderr = stderr.to_ascii_lowercase();
            stderr.contains("no image data") || stderr.contains("clipboard does not contain an image")
        })
}

fn wl_type_unavailable(error: &RunError) -> bool {
    matches!(error, RunError::NoData)
        || matches!(error, RunError::CommandFailed { stderr, .. } if {
            let stderr = stderr.to_ascii_lowercase();
            stderr.contains("type") && (stderr.contains("not available") || stderr.contains("not offered"))
        })
}

fn preferred_image_mime(types: &str) -> Option<&'static str> {
    if types.lines().any(|mime| mime == "image/png") {
        Some("image/png")
    } else if types.lines().any(|mime| mime == "image/jpeg") {
        Some("image/jpeg")
    } else {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RunError {
    NoData,
    Unavailable,
    Cancelled,
    Timeout,
    TooLarge,
    CommandFailed { code: Option<i32>, stderr: String },
    ReadFailed,
}

fn run_bounded_with_limit(
    program: &str,
    args: &[&str],
    cancel: &AtomicBool,
    max_bytes: u64,
) -> Result<Vec<u8>, RunError> {
    #[cfg(unix)]
    {
        run_bounded_unix(program, args, cancel, max_bytes)
    }
    #[cfg(not(unix))]
    {
        run_bounded_threaded(program, args, cancel, max_bytes)
    }
}

#[cfg(unix)]
fn run_bounded_unix(
    program: &str,
    args: &[&str],
    cancel: &AtomicBool,
    max_bytes: u64,
) -> Result<Vec<u8>, RunError> {
    use std::os::fd::AsRawFd;
    use std::os::unix::process::CommandExt;

    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // SAFETY: setpgid is async-signal-safe and this closure does not access parent memory.
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        });
    }
    let child = command.spawn().map_err(spawn_error)?;
    let mut process = OwnedChildProcess::new(child);
    let mut stdout = process
        .child_mut()
        .stdout
        .take()
        .ok_or(RunError::ReadFailed)?;
    let mut stderr = process
        .child_mut()
        .stderr
        .take()
        .ok_or(RunError::ReadFailed)?;
    set_nonblocking(stdout.as_raw_fd())?;
    set_nonblocking(stderr.as_raw_fd())?;

    let mut output = BoundedOutput::new(max_bytes);
    let mut diagnostic = BoundedOutput::new(MAX_DIAGNOSTIC_BYTES);
    let mut stdout_eof = false;
    let mut stderr_eof = false;
    let deadline = Instant::now() + CLIPBOARD_TIMEOUT;
    let status = loop {
        stdout_eof |= output.read_available(&mut stdout)?;
        stderr_eof |= diagnostic.read_available(&mut stderr)?;
        if cancel.load(Ordering::Acquire) {
            process.terminate_and_wait()?;
            return Err(RunError::Cancelled);
        }
        if Instant::now() >= deadline {
            process.terminate_and_wait()?;
            return Err(RunError::Timeout);
        }
        if let Some(status) = process.try_wait()? {
            break status;
        }
        thread::sleep(Duration::from_millis(10));
    };

    // The leader has been reaped, so never signal its numeric process-group id again: it can be
    // reused. Descendants that inherited a pipe are handled only by a bounded drain followed by
    // closing our readers.
    let drain_deadline = Instant::now() + Duration::from_millis(250);
    while Instant::now() < drain_deadline {
        stdout_eof |= output.read_available(&mut stdout)?;
        stderr_eof |= diagnostic.read_available(&mut stderr)?;
        if stdout_eof && stderr_eof {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    finish_output(status, output, diagnostic)
}

#[cfg(unix)]
fn set_nonblocking(fd: std::os::fd::RawFd) -> Result<(), RunError> {
    // SAFETY: fd is an open pipe owned by this function's caller.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 {
        return Err(RunError::ReadFailed);
    }
    // SAFETY: fd remains open and F_SETFL only updates its file status flags.
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(RunError::ReadFailed);
    }
    Ok(())
}

#[cfg(unix)]
fn signal_process_group(process_group: i32, signal: i32) {
    // SAFETY: callers retain the unreaped leader of the private process group, preventing reuse of
    // its pid as a process-group id while the signal is sent.
    unsafe {
        libc::kill(-process_group, signal);
    }
}

#[cfg(unix)]
struct OwnedChildProcess {
    child: std::process::Child,
    process_group: Option<i32>,
    leader_reaped: bool,
}

#[cfg(unix)]
impl OwnedChildProcess {
    fn new(child: std::process::Child) -> Self {
        let process_group = i32::try_from(child.id()).ok();
        Self {
            child,
            process_group,
            leader_reaped: false,
        }
    }

    fn child_mut(&mut self) -> &mut std::process::Child {
        &mut self.child
    }

    fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>, RunError> {
        let status = self.child.try_wait().map_err(wait_error)?;
        if status.is_some() {
            self.leader_reaped = true;
        }
        Ok(status)
    }

    fn terminate_and_wait(&mut self) -> Result<(), RunError> {
        if self.leader_reaped {
            return Ok(());
        }
        if let Some(process_group) = self.process_group {
            signal_process_group(process_group, libc::SIGTERM);
            let grace_deadline = Instant::now() + PROCESS_GROUP_TERM_GRACE;
            while Instant::now() < grace_deadline {
                // Do not reap yet: the leader reserves its numeric pgid until the final signal.
                thread::sleep(Duration::from_millis(5));
            }
            signal_process_group(process_group, libc::SIGKILL);
        } else {
            // A real Unix pid fits pid_t. Keep this fallback so even an unexpected conversion
            // failure cannot leak the directly owned child.
            let _ = self.child.kill();
        }
        self.child.wait().map_err(wait_error)?;
        self.leader_reaped = true;
        Ok(())
    }
}

#[cfg(unix)]
impl Drop for OwnedChildProcess {
    fn drop(&mut self) {
        // Every `?` after spawn passes through here. Once try_wait observed an exit, this is a
        // no-op, so a numeric pgid is never signalled after its leader has been reaped.
        let _ = self.terminate_and_wait();
    }
}

#[cfg(unix)]
struct BoundedOutput {
    bytes: Vec<u8>,
    limit: usize,
    too_large: bool,
}

#[cfg(unix)]
impl BoundedOutput {
    fn new(max_bytes: u64) -> Self {
        let limit = usize::try_from(max_bytes).unwrap_or(usize::MAX);
        Self {
            bytes: Vec::with_capacity(limit.min(64 * 1024)),
            limit,
            too_large: false,
        }
    }

    #[cfg(unix)]
    fn read_available(&mut self, reader: &mut impl Read) -> Result<bool, RunError> {
        let mut buffer = [0_u8; 8192];
        for _ in 0..MAX_READS_PER_POLL {
            match reader.read(&mut buffer) {
                Ok(0) => return Ok(true),
                Ok(read) => self.push(&buffer[..read]),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => return Err(RunError::ReadFailed),
            }
        }
        Ok(false)
    }

    fn push(&mut self, bytes: &[u8]) {
        let retained = self.limit.saturating_sub(self.bytes.len()).min(bytes.len());
        self.bytes.extend_from_slice(&bytes[..retained]);
        self.too_large |= retained < bytes.len();
    }
}

fn spawn_error(error: std::io::Error) -> RunError {
    if error.kind() == std::io::ErrorKind::NotFound {
        RunError::Unavailable
    } else {
        RunError::CommandFailed {
            code: None,
            stderr: sanitize_diagnostic(&error.to_string()),
        }
    }
}

fn wait_error(error: std::io::Error) -> RunError {
    RunError::CommandFailed {
        code: None,
        stderr: sanitize_diagnostic(&error.to_string()),
    }
}

#[cfg(unix)]
fn finish_output(
    status: std::process::ExitStatus,
    output: BoundedOutput,
    diagnostic: BoundedOutput,
) -> Result<Vec<u8>, RunError> {
    if !status.success() {
        return Err(RunError::CommandFailed {
            code: status.code(),
            stderr: sanitize_diagnostic(&String::from_utf8_lossy(&diagnostic.bytes)),
        });
    }
    if output.too_large {
        return Err(RunError::TooLarge);
    }
    if output.bytes.is_empty() {
        return Err(RunError::NoData);
    }
    Ok(output.bytes)
}

#[cfg(not(unix))]
fn run_bounded_threaded(
    program: &str,
    args: &[&str],
    cancel: &AtomicBool,
    max_bytes: u64,
) -> Result<Vec<u8>, RunError> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(spawn_error)?;
    let stdout = child.stdout.take().ok_or(RunError::ReadFailed)?;
    let stderr = child.stderr.take().ok_or(RunError::ReadFailed)?;
    let reader = thread::spawn(move || read_bounded_and_drain(stdout, max_bytes));
    let stderr_reader = thread::spawn(move || read_bounded_and_drain(stderr, MAX_DIAGNOSTIC_BYTES));
    let deadline = Instant::now() + CLIPBOARD_TIMEOUT;
    let status = loop {
        match child.try_wait().map_err(|error| RunError::CommandFailed {
            code: None,
            stderr: sanitize_diagnostic(&error.to_string()),
        })? {
            Some(status) => break status,
            None if Instant::now() >= deadline || cancel.load(Ordering::Acquire) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                let _ = stderr_reader.join();
                return Err(if cancel.load(Ordering::Acquire) {
                    RunError::Cancelled
                } else {
                    RunError::Timeout
                });
            }
            None => thread::sleep(Duration::from_millis(10)),
        }
    };
    let (bytes, output_too_large) = reader
        .join()
        .map_err(|_| RunError::ReadFailed)?
        .map_err(|_| RunError::ReadFailed)?;
    let (stderr, _) = stderr_reader
        .join()
        .map_err(|_| RunError::ReadFailed)?
        .map_err(|_| RunError::ReadFailed)?;
    if !status.success() {
        return Err(RunError::CommandFailed {
            code: status.code(),
            stderr: sanitize_diagnostic(&String::from_utf8_lossy(&stderr)),
        });
    }
    if output_too_large {
        return Err(RunError::TooLarge);
    }
    if bytes.is_empty() {
        return Err(RunError::NoData);
    }
    Ok(bytes)
}

#[cfg(not(unix))]
fn read_bounded_and_drain(
    mut reader: impl Read,
    max_bytes: u64,
) -> std::io::Result<(Vec<u8>, bool)> {
    let capacity = usize::try_from(max_bytes).unwrap_or(usize::MAX);
    let mut captured = Vec::with_capacity(capacity.min(64 * 1024));
    let mut too_large = false;
    let mut buffer = [0_u8; 8192];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let remaining = capacity.saturating_sub(captured.len());
        let retained = remaining.min(read);
        captured.extend_from_slice(&buffer[..retained]);
        too_large |= retained < read;
    }
    Ok((captured, too_large))
}

impl std::fmt::Display for RunError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoData => write!(formatter, "no data"),
            Self::Unavailable => write!(formatter, "command unavailable"),
            Self::Cancelled => write!(formatter, "cancelled"),
            Self::Timeout => write!(formatter, "timed out"),
            Self::TooLarge => write!(formatter, "output exceeds configured limit"),
            Self::CommandFailed { code, stderr } => {
                write!(formatter, "command exited with status {code:?}")?;
                if !stderr.is_empty() {
                    write!(formatter, ": {stderr}")?;
                }
                Ok(())
            }
            Self::ReadFailed => write!(formatter, "failed to read command output"),
        }
    }
}

fn sanitize_diagnostic(value: &str) -> String {
    let normalized = value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut end = normalized.len().min(MAX_DIAGNOSTIC_BYTES as usize);
    while !normalized.is_char_boundary(end) {
        end -= 1;
    }
    normalized[..end].to_string()
}

fn detect_image(bytes: &[u8]) -> Result<(&'static str, &'static str)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Ok(("PNG", "png"))
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Ok(("JPEG", "jpg"))
    } else {
        bail!("clipboard content is not a supported PNG or JPEG image")
    }
}

#[cfg(unix)]
fn create_private_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    let mut builder = fs::DirBuilder::new();
    builder
        .mode(0o700)
        .create(path)
        .with_context(|| format!("failed to create {}", path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn create_private_directory(path: &Path) -> Result<()> {
    fs::create_dir(path).with_context(|| format!("failed to create {}", path.display()))?;
    Ok(())
}

#[cfg(unix)]
fn set_private_file_options(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600);
}

#[cfg(not(unix))]
fn set_private_file_options(_options: &mut OpenOptions) {}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::Ordering;
    use std::thread;
    use std::time::Duration;
    use std::time::Instant;
    use std::time::SystemTime;

    use super::ClipboardImages;
    use super::DIRECTORY_PREFIX;
    use super::detect_image;

    struct ExpectedRun {
        program: &'static str,
        args: &'static [&'static str],
        max_bytes: u64,
        result: Result<Vec<u8>, super::RunError>,
    }

    struct FakeRunner {
        runs: RefCell<VecDeque<ExpectedRun>>,
    }

    impl FakeRunner {
        fn new(runs: Vec<ExpectedRun>) -> Self {
            Self {
                runs: RefCell::new(runs.into()),
            }
        }

        fn done(&self) {
            assert!(self.runs.borrow().is_empty());
        }
    }

    impl super::Runner for FakeRunner {
        fn run(
            &self,
            program: &str,
            args: &[&str],
            max_bytes: u64,
        ) -> Result<Vec<u8>, super::RunError> {
            let expected = self.runs.borrow_mut().pop_front().expect("unexpected run");
            assert_eq!(program, expected.program);
            assert_eq!(args, expected.args);
            assert_eq!(max_bytes, expected.max_bytes);
            expected.result
        }
    }

    fn run(
        program: &'static str,
        args: &'static [&'static str],
        max_bytes: u64,
        result: Result<&'static [u8], super::RunError>,
    ) -> ExpectedRun {
        ExpectedRun {
            program,
            args,
            max_bytes,
            result: result.map(<[u8]>::to_vec),
        }
    }

    fn failed(stderr: &str) -> super::RunError {
        super::RunError::CommandFailed {
            code: Some(1),
            stderr: stderr.to_string(),
        }
    }

    #[test]
    fn detects_supported_image_signatures() {
        assert_eq!(
            detect_image(b"\x89PNG\r\n\x1a\nrest").unwrap(),
            ("PNG", "png")
        );
        assert_eq!(
            detect_image(&[0xff, 0xd8, 0xff, 0xdb]).unwrap(),
            ("JPEG", "jpg")
        );
        assert!(detect_image(b"plain text").is_err());
    }

    #[test]
    fn selects_supported_image_mime_before_text() {
        assert_eq!(
            super::preferred_image_mime("text/plain\nimage/jpeg\nimage/png\n"),
            Some("image/png")
        );
        assert_eq!(
            super::preferred_image_mime("text/plain\nimage/jpeg\n"),
            Some("image/jpeg")
        );
        assert_eq!(super::preferred_image_mime("text/plain\n"), None);
    }

    #[test]
    fn text_decoding_has_parameterized_limit_and_requires_utf8() {
        assert_eq!(super::decode_text(b"hello".to_vec(), 5).unwrap(), "hello");
        assert!(super::decode_text(b"hello".to_vec(), 4).is_err());
        assert!(super::decode_text(vec![0xff], 1).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn bounded_runner_reports_typed_outcomes() {
        let cancel = AtomicBool::new(false);
        assert_eq!(
            super::run_bounded_with_limit("sh", &["-c", "exit 0"], &cancel, 10),
            Err(super::RunError::NoData)
        );
        assert_eq!(
            super::run_bounded_with_limit("sh", &["-c", "printf 12345"], &cancel, 4),
            Err(super::RunError::TooLarge)
        );
        assert_eq!(
            super::run_bounded_with_limit("sh", &["-c", "exit 7"], &cancel, 10),
            Err(super::RunError::CommandFailed {
                code: Some(7),
                stderr: String::new(),
            })
        );
    }

    #[cfg(unix)]
    #[test]
    fn direct_child_exit_with_descendant_holding_pipes_returns_promptly() {
        let cancel = AtomicBool::new(false);
        let started = Instant::now();
        let result =
            super::run_bounded_with_limit("sh", &["-c", "sleep 10 & printf ready"], &cancel, 10);

        assert_eq!(result, Ok(b"ready".to_vec()));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[cfg(unix)]
    #[test]
    fn continuously_writable_pipe_cannot_bypass_timeout() {
        let cancel = AtomicBool::new(false);
        let started = Instant::now();
        let result = super::run_bounded_with_limit(
            "sh",
            &[
                "-c",
                "while :; do printf 12345678901234567890123456789012; done",
            ],
            &cancel,
            10,
        );

        assert_eq!(result, Err(super::RunError::Timeout));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[cfg(unix)]
    #[test]
    fn owned_child_guard_reaps_process_when_later_step_fails() {
        use std::os::unix::process::CommandExt;

        let mut command = std::process::Command::new("sleep");
        command.arg("10");
        // SAFETY: setpgid is async-signal-safe and this closure does not access parent memory.
        unsafe {
            command.pre_exec(|| {
                if libc::setpgid(0, 0) == 0 {
                    Ok(())
                } else {
                    Err(std::io::Error::last_os_error())
                }
            });
        }
        let child = command.spawn().unwrap();
        let pid = i32::try_from(child.id()).unwrap();
        let result = {
            let _process = super::OwnedChildProcess::new(child);
            Err::<(), _>(super::RunError::ReadFailed)
        };

        assert_eq!(result, Err(super::RunError::ReadFailed));
        // SAFETY: signal 0 only checks whether this numeric pid still names a process.
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reaped_leader_with_setsid_descendant_holding_pipe_returns_promptly() {
        let cancel = AtomicBool::new(false);
        let started = Instant::now();
        let result = super::run_bounded_with_limit(
            "sh",
            &["-c", "setsid sh -c 'sleep 10' & printf ready"],
            &cancel,
            10,
        );

        assert_eq!(result, Ok(b"ready".to_vec()));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn wayland_orchestration_lists_types_then_reads_image_or_text_with_exact_caps() {
        let image = FakeRunner::new(vec![
            run(
                "wl-paste",
                &["--list-types"],
                super::MAX_TEXT_BYTES as u64,
                Ok(b"text/plain\nimage/png\n"),
            ),
            run(
                "wl-paste",
                &["--no-newline", "--type", "image/png"],
                super::MAX_IMAGE_BYTES,
                Ok(b"\x89PNG\r\n\x1a\nbody"),
            ),
        ]);
        assert!(matches!(
            super::capture_clipboard(&image, super::Backend::Wayland).unwrap(),
            super::CapturedClipboard::Image(_)
        ));
        image.done();

        let text = FakeRunner::new(vec![
            run(
                "wl-paste",
                &["--list-types"],
                super::MAX_TEXT_BYTES as u64,
                Ok(b"text/plain\n"),
            ),
            run(
                "wl-paste",
                &["--no-newline", "--type", "text/plain;charset=utf-8"],
                super::MAX_TEXT_BYTES as u64,
                Err(failed("type not offered")),
            ),
            run(
                "wl-paste",
                &["--no-newline", "--type", "text/plain"],
                super::MAX_TEXT_BYTES as u64,
                Ok(b"hello"),
            ),
        ]);
        assert!(matches!(
            super::capture_clipboard(&text, super::Backend::Wayland).unwrap(),
            super::CapturedClipboard::Text(value) if value == "hello"
        ));
        text.done();
    }

    #[test]
    fn wayland_list_image_and_text_operational_errors_are_hard_errors() {
        for runs in [
            vec![run(
                "wl-paste",
                &["--list-types"],
                super::MAX_TEXT_BYTES as u64,
                Err(failed("permission denied")),
            )],
            vec![
                run(
                    "wl-paste",
                    &["--list-types"],
                    super::MAX_TEXT_BYTES as u64,
                    Ok(b"image/png\n"),
                ),
                run(
                    "wl-paste",
                    &["--no-newline", "--type", "image/png"],
                    super::MAX_IMAGE_BYTES,
                    Err(failed("compositor disconnected")),
                ),
            ],
            vec![
                run(
                    "wl-paste",
                    &["--list-types"],
                    super::MAX_TEXT_BYTES as u64,
                    Ok(b"text/plain\n"),
                ),
                run(
                    "wl-paste",
                    &["--no-newline", "--type", "text/plain;charset=utf-8"],
                    super::MAX_TEXT_BYTES as u64,
                    Err(failed("permission denied")),
                ),
            ],
        ] {
            let runner = FakeRunner::new(runs);
            assert!(super::capture_clipboard(&runner, super::Backend::Wayland).is_err());
            runner.done();
        }
    }

    #[test]
    fn x11_falls_back_only_for_type_unavailable_then_reads_text() {
        let image = FakeRunner::new(vec![run(
            "xclip",
            &["-selection", "clipboard", "-t", "image/png", "-o"],
            super::MAX_IMAGE_BYTES,
            Ok(b"\x89PNG\r\n\x1a\nbody"),
        )]);
        assert!(matches!(
            super::capture_clipboard(&image, super::Backend::X11).unwrap(),
            super::CapturedClipboard::Image(_)
        ));
        image.done();

        let runner = FakeRunner::new(vec![
            run(
                "xclip",
                &["-selection", "clipboard", "-t", "image/png", "-o"],
                super::MAX_IMAGE_BYTES,
                Err(failed("Error: target image/png not available")),
            ),
            run(
                "xclip",
                &["-selection", "clipboard", "-t", "image/jpeg", "-o"],
                super::MAX_IMAGE_BYTES,
                Err(failed("Error: target image/jpeg not available")),
            ),
            run(
                "xclip",
                &["-selection", "clipboard", "-o"],
                super::MAX_TEXT_BYTES as u64,
                Ok(b"x text"),
            ),
        ]);
        assert!(matches!(
            super::capture_clipboard(&runner, super::Backend::X11).unwrap(),
            super::CapturedClipboard::Text(value) if value == "x text"
        ));
        runner.done();

        let operational = FakeRunner::new(vec![run(
            "xclip",
            &["-selection", "clipboard", "-t", "image/png", "-o"],
            super::MAX_IMAGE_BYTES,
            Err(failed("Error: Can't open display")),
        )]);
        assert!(super::capture_clipboard(&operational, super::Backend::X11).is_err());
        operational.done();
    }

    #[test]
    fn macos_falls_back_for_documented_no_image_but_not_operational_failure() {
        let image = FakeRunner::new(vec![run(
            "pngpaste",
            &["-"],
            super::MAX_IMAGE_BYTES,
            Ok(b"\x89PNG\r\n\x1a\nbody"),
        )]);
        assert!(matches!(
            super::capture_clipboard(&image, super::Backend::MacOs).unwrap(),
            super::CapturedClipboard::Image(_)
        ));
        image.done();

        let runner = FakeRunner::new(vec![
            run(
                "pngpaste",
                &["-"],
                super::MAX_IMAGE_BYTES,
                Err(failed("No image data found on the clipboard.")),
            ),
            run(
                "pbpaste",
                &[],
                super::MAX_TEXT_BYTES as u64,
                Ok(b"mac text"),
            ),
        ]);
        assert!(matches!(
            super::capture_clipboard(&runner, super::Backend::MacOs).unwrap(),
            super::CapturedClipboard::Text(value) if value == "mac text"
        ));
        runner.done();

        let operational = FakeRunner::new(vec![run(
            "pngpaste",
            &["-"],
            super::MAX_IMAGE_BYTES,
            Err(failed("pasteboard access denied")),
        )]);
        assert!(super::capture_clipboard(&operational, super::Backend::MacOs).is_err());
        operational.done();
    }

    #[test]
    fn missing_wayland_tool_is_not_silently_replaced_with_xclip() {
        let runner = FakeRunner::new(vec![run(
            "wl-paste",
            &["--list-types"],
            super::MAX_TEXT_BYTES as u64,
            Err(super::RunError::Unavailable),
        )]);
        assert!(super::capture_clipboard(&runner, super::Backend::Wayland).is_err());
        runner.done();
    }

    #[test]
    fn diagnostics_are_bounded_and_sanitized() {
        let diagnostic = format!(
            "secret\n{}",
            "x".repeat(super::MAX_DIAGNOSTIC_BYTES as usize + 20)
        );
        let sanitized = super::sanitize_diagnostic(&diagnostic);
        assert!(!sanitized.contains('\n'));
        assert!(sanitized.len() <= super::MAX_DIAGNOSTIC_BYTES as usize);
    }

    #[cfg(unix)]
    #[test]
    fn creates_private_directory_atomically_and_removes_it_on_drop() {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!("ccm-clipboard-test-{}", std::process::id()));
        super::create_private_directory(&root).unwrap();
        let directory = {
            let mut images = ClipboardImages::new_in(root.clone(), SystemTime::now());
            let captured = super::CapturedImage {
                bytes: b"\x89PNG\r\n\x1a\nrest".to_vec(),
                format: "PNG",
                extension: "png",
            };
            images.store(captured).unwrap();
            assert_eq!(
                fs::metadata(&images.directory)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            images.directory.clone()
        };
        assert!(!directory.exists());
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn startup_removes_only_stale_owned_directories() {
        let root = std::env::temp_dir().join(format!("ccm-stale-test-{}", std::process::id()));
        super::create_private_directory(&root).unwrap();
        let stale = root.join(format!("{DIRECTORY_PREFIX}stale"));
        let unrelated = root.join("unrelated");
        fs::create_dir(&stale).unwrap();
        fs::create_dir(&unrelated).unwrap();
        let future = SystemTime::now() + Duration::from_secs(8 * 24 * 60 * 60);

        let images = ClipboardImages::new_in(root.clone(), future);

        assert!(!stale.exists());
        assert!(unrelated.exists());
        drop(images);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn clipboard_root_is_scoped_below_the_supplied_temp_directory() {
        let temp = PathBuf::from("/tmp/example-temp");
        let root = super::clipboard_root(&temp);
        assert_eq!(root.parent(), Some(temp.as_path()));
        assert!(
            root.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("codex-claude-mode-clipboard-"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_kills_and_waits_for_clipboard_child_promptly() {
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let started = Instant::now();
        let worker = thread::spawn(move || {
            super::run_bounded_with_limit("sleep", &["10"], &worker_cancel, 10)
        });
        thread::sleep(Duration::from_millis(30));
        cancel.store(true, Ordering::Release);

        assert!(worker.join().unwrap().is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
