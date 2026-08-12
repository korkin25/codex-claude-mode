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
const CLIPBOARD_TIMEOUT: Duration = Duration::from_secs(3);
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

pub(crate) struct ClipboardImages {
    root: PathBuf,
    directory: PathBuf,
    next_number: usize,
}

pub(crate) struct ClipboardCapture {
    receiver: Receiver<Result<CapturedImage>>,
    cancel: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl ClipboardCapture {
    pub(crate) fn try_recv(&self) -> Result<Result<CapturedImage>, mpsc::TryRecvError> {
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
            let result = capture_image_bytes(&worker_cancel).and_then(|bytes| {
                let (format, extension) = detect_image(&bytes)?;
                Ok(CapturedImage {
                    bytes,
                    format,
                    extension,
                })
            });
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

fn capture_image_bytes(cancel: &AtomicBool) -> Result<Vec<u8>> {
    let candidates: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pngpaste", &["-"])]
    } else {
        &[
            ("wl-paste", &["--no-newline", "--type", "image/png"]),
            ("wl-paste", &["--no-newline", "--type", "image/jpeg"]),
            (
                "xclip",
                &["-selection", "clipboard", "-t", "image/png", "-o"],
            ),
            (
                "xclip",
                &["-selection", "clipboard", "-t", "image/jpeg", "-o"],
            ),
        ]
    };
    let mut available = false;
    for (program, args) in candidates {
        if cancel.load(Ordering::Acquire) {
            bail!("clipboard image capture cancelled")
        }
        match run_bounded(program, args, cancel) {
            Ok(bytes) if !bytes.is_empty() => return Ok(bytes),
            Ok(_) => available = true,
            Err(RunError::Unavailable) => {}
            Err(RunError::Failed) => available = true,
        }
    }
    if available {
        bail!("clipboard does not contain a readable PNG or the clipboard command failed")
    }
    if cfg!(target_os = "macos") {
        bail!("image clipboard unavailable: install pngpaste")
    }
    bail!("image clipboard unavailable: install wl-clipboard or xclip")
}

enum RunError {
    Unavailable,
    Failed,
}

fn run_bounded(program: &str, args: &[&str], cancel: &AtomicBool) -> Result<Vec<u8>, RunError> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                RunError::Unavailable
            } else {
                RunError::Failed
            }
        })?;
    let stdout = child.stdout.take().ok_or(RunError::Failed)?;
    let reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take(MAX_IMAGE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let deadline = Instant::now() + CLIPBOARD_TIMEOUT;
    let status = loop {
        match child.try_wait().map_err(|_| RunError::Failed)? {
            Some(status) => break status,
            None if Instant::now() >= deadline || cancel.load(Ordering::Acquire) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err(RunError::Failed);
            }
            None => thread::sleep(Duration::from_millis(10)),
        }
    };
    let bytes = reader
        .join()
        .map_err(|_| RunError::Failed)?
        .map_err(|_| RunError::Failed)?;
    if !status.success() || bytes.len() as u64 > MAX_IMAGE_BYTES {
        return Err(RunError::Failed);
    }
    Ok(bytes)
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
        let worker = thread::spawn(move || super::run_bounded("sleep", &["10"], &worker_cancel));
        thread::sleep(Duration::from_millis(30));
        cancel.store(true, Ordering::Release);

        assert!(worker.join().unwrap().is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
