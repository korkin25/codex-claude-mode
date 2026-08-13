use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use pretty_assertions::assert_eq;
use serde_json::Value;
use serde_json::json;

use crate::App;
use crate::DescendantChain;
use crate::ListChain;
use crate::MAX_LIST_ITEMS;
use crate::MAX_LIST_PAGES;
use crate::Pending;
use crate::backend::Backend;
use crate::clipboard::ClipboardImages;
use crate::ui::SessionSelection;
use crate::ui::Submission;
use crate::ui::SubmissionInput;
use crate::ui::Workspace;

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "codex-claude-mode-transcript-{}-{id}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create transcript test directory");
        Self(path)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        for _ in 0..50 {
            match fs::remove_dir_all(&self.0) {
                Ok(()) => return,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
                Err(_) => thread::sleep(Duration::from_millis(2)),
            }
        }
        fs::remove_dir_all(&self.0).expect("remove transcript test directory");
    }
}

struct Harness {
    _directory: TestDir,
    requests: PathBuf,
    app: App,
}

impl Harness {
    fn new(cwd: &Path, preferred_root: Option<&str>) -> Self {
        let directory = TestDir::new();
        let executable = directory.0.join("fake-codex");
        let requests = directory.0.join("requests.jsonl");
        fs::write(
            &executable,
            format!(
                "#!/bin/sh\nwhile IFS= read -r line; do printf '%s\\n' \"$line\" >> '{}'; done\n",
                requests.display()
            ),
        )
        .expect("write fake app-server");
        let mut permissions = fs::metadata(&executable)
            .expect("read fake app-server metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&executable, permissions).expect("make fake app-server executable");
        thread::sleep(Duration::from_millis(10));
        let codex_home = directory.0.join("codex-home");
        let backend = Backend::spawn(&executable, &codex_home, &[]).expect("spawn fake app-server");
        let mut workspace = Workspace::new();
        workspace.set_completion_cwd(cwd.to_path_buf());
        let preferred_root = preferred_root.map(str::to_owned);
        let app = App {
            backend,
            workspace,
            pending: HashMap::new(),
            loaded_history: HashSet::new(),
            live_items: HashMap::new(),
            preferred_root: preferred_root.clone(),
            session_decided: preferred_root.is_some(),
            starting_new_session: false,
            cwd: cwd.to_path_buf(),
            active_session_cwd: cwd.to_path_buf(),
            last_refresh: Instant::now(),
            permission_profiles: HashMap::new(),
            codex: executable,
            codex_home,
            update_result: None,
            clipboard_images: ClipboardImages::new(),
            clipboard_capture: None,
            clipboard_target: None,
            list_generation: 0,
            list_chain: None,
            descendants_generation: 0,
            descendants_chain: None,
            codex_home_was_empty: false,
        };
        Self {
            _directory: directory,
            requests,
            app,
        }
    }

    fn requests(&self) -> Vec<Value> {
        for _ in 0..50 {
            if self.requests.exists() {
                let contents = fs::read_to_string(&self.requests).expect("read requests");
                if !contents.is_empty() {
                    return contents
                        .lines()
                        .map(|line| serde_json::from_str(line).expect("valid request JSON"))
                        .collect();
                }
            }
            thread::sleep(Duration::from_millis(2));
        }
        Vec::new()
    }

    fn methods(&self) -> Vec<String> {
        self.requests()
            .iter()
            .filter_map(|request| request.get("method").and_then(Value::as_str))
            .map(str::to_owned)
            .collect()
    }
}

fn thread_value(id: &str, cwd: &Path) -> Value {
    json!({
        "id": id,
        "cwd": cwd,
        "preview": id,
        "createdAt": 1,
        "updatedAt": 1
    })
}

#[test]
fn explicit_thread_resumes_directly_with_current_cwd_and_never_lists_or_starts() {
    let cwd = Path::new("/work/current");
    let mut harness = Harness::new(cwd, Some("known"));

    harness
        .app
        .handle_response(
            Pending::Initialize,
            &json!({"result": {"userAgent": "fake"}}),
        )
        .expect("handle initialize");

    let requests = harness.requests();
    let resume = requests
        .iter()
        .find(|request| request["method"] == "thread/resume")
        .expect("resume request");
    assert_eq!(resume["params"], json!({"threadId": "known", "cwd": cwd}));
    assert!(
        !harness
            .methods()
            .iter()
            .any(|method| method == "thread/list")
    );
    assert!(
        !harness
            .methods()
            .iter()
            .any(|method| method == "thread/start")
    );
}

#[test]
fn explicit_unknown_thread_is_visible_and_never_starts_a_replacement() {
    let mut harness = Harness::new(Path::new("/work/current"), Some("missing"));

    harness
        .app
        .handle_response(
            Pending::OpenThread {
                thread_id: "missing".to_string(),
                resume_cwd: PathBuf::from("/work/current"),
            },
            &json!({"error": {"code": -32000, "message": "not found"}}),
        )
        .expect("handle missing thread");

    assert!(
        harness
            .app
            .workspace
            .status_line
            .contains("could not open thread missing")
    );
    assert!(
        !harness
            .methods()
            .iter()
            .any(|method| method == "thread/start")
    );
    assert!(harness.app.session_decided);
    assert!(harness.app.preferred_root.is_none());
}

#[test]
fn explicit_thread_malformed_success_stays_failed_until_user_action() {
    let mut harness = Harness::new(Path::new("/work/current"), Some("malformed"));

    harness
        .app
        .handle_response(
            Pending::OpenThread {
                thread_id: "malformed".to_string(),
                resume_cwd: PathBuf::from("/work/current"),
            },
            &json!({"result": {"thread": {"cwd": "/work/current"}}}),
        )
        .expect("handle malformed thread response");

    assert_eq!(
        harness.app.workspace.status_line,
        "thread malformed was not found"
    );
    assert!(harness.app.session_decided);
    assert!(harness.app.preferred_root.is_none());
    assert!(
        !harness
            .methods()
            .iter()
            .any(|method| { matches!(method.as_str(), "thread/list" | "thread/start") })
    );
}

#[test]
fn root_pagination_accumulates_more_than_one_page() {
    let mut harness = Harness::new(Path::new("/work/current"), None);
    harness
        .app
        .request_list(true, None)
        .expect("request first page");
    let generation = harness.app.list_generation;
    let first = (0..200)
        .map(|index| thread_value(&format!("root-{index}"), Path::new("/saved")))
        .collect::<Vec<_>>();
    harness
        .app
        .apply_list(&json!({"data": first, "nextCursor": "page-2"}), generation)
        .expect("apply first page");
    harness
        .app
        .apply_list(
            &json!({
                "data": [thread_value("root-200", Path::new("/saved"))],
                "nextCursor": "page-3"
            }),
            generation,
        )
        .expect("apply second page");

    assert_eq!(
        harness
            .app
            .list_chain
            .as_ref()
            .expect("active pagination chain")
            .threads
            .len(),
        201
    );
}

#[test]
fn descendant_pagination_accumulates_more_than_one_page() {
    let mut harness = Harness::new(Path::new("/work/current"), Some("root"));
    harness
        .app
        .upsert_thread(&thread_value("root", Path::new("/work/current")));
    harness.app.workspace.rebuild_tree(Some("root"));
    harness
        .app
        .request_descendants(None)
        .expect("request first page");
    let generation = harness.app.descendants_generation;
    let first = (0..200)
        .map(|index| {
            json!({"id": format!("child-{index}"), "parentThreadId": "root", "createdAt": index})
        })
        .collect::<Vec<_>>();
    harness
        .app
        .apply_descendants(
            "root",
            generation,
            &json!({"data": first, "nextCursor": "page-2"}),
        )
        .expect("apply first page");
    harness
        .app
        .apply_descendants(
            "root",
            generation,
            &json!({"data": [{"id": "child-200", "parentThreadId": "root"}]}),
        )
        .expect("apply second page");

    assert_eq!(harness.app.workspace.order.len(), 202);
}

#[test]
fn stale_normal_and_recovery_pages_cannot_replace_the_newer_generation() {
    let mut harness = Harness::new(Path::new("/work/current"), None);
    harness.app.request_list(false, None).expect("normal list");
    let stale_generation = harness.app.list_generation;
    harness.app.request_list(true, None).expect("recovery list");
    let current_generation = harness.app.list_generation;

    harness
        .app
        .apply_list(
            &json!({"data": [thread_value("stale", Path::new("/work/current"))]}),
            stale_generation,
        )
        .expect("ignore stale page");
    harness
        .app
        .apply_list(
            &json!({
                "data": [thread_value("current", Path::new("/saved"))],
                "nextCursor": "current-page-2"
            }),
            current_generation,
        )
        .expect("apply current page");

    let chain = harness.app.list_chain.as_ref().expect("current chain");
    assert_eq!(chain.threads.len(), 1);
    assert_eq!(chain.threads[0]["id"], "current");
}

#[test]
fn repeated_cursor_and_list_bounds_fail_explicitly() {
    let mut harness = Harness::new(Path::new("/work/current"), None);
    harness.app.request_list(true, None).expect("request list");
    let generation = harness.app.list_generation;
    harness
        .app
        .apply_list(&json!({"data": [], "nextCursor": "same"}), generation)
        .expect("apply first cursor");
    harness
        .app
        .apply_list(&json!({"data": [], "nextCursor": "same"}), generation)
        .expect("reject repeated cursor");
    assert!(
        harness
            .app
            .workspace
            .status_line
            .contains("repeated a pagination cursor")
    );

    harness.app.list_chain = Some(ListChain {
        generation,
        all_workspaces: true,
        pages: MAX_LIST_PAGES,
        cursors: HashSet::new(),
        ids: HashSet::new(),
        threads: Vec::new(),
    });
    harness
        .app
        .apply_list(&json!({"data": []}), generation)
        .expect("reject page overflow");
    assert!(
        harness
            .app
            .workspace
            .status_line
            .contains("exceeded 100 pages")
    );

    let ids = (0..=MAX_LIST_ITEMS)
        .map(|index| format!("root-item-{index}"))
        .collect::<HashSet<_>>();
    let threads = ids
        .iter()
        .map(|id| thread_value(id, Path::new("/saved")))
        .collect();
    harness.app.list_chain = Some(ListChain {
        generation,
        all_workspaces: true,
        pages: 0,
        cursors: HashSet::new(),
        ids,
        threads,
    });
    harness
        .app
        .apply_list(
            &json!({"data": [thread_value("root-overflow", Path::new("/saved"))]}),
            generation,
        )
        .expect("reject root item overflow");
    assert!(
        harness
            .app
            .workspace
            .status_line
            .contains("exceeded 20000 unique threads")
    );
}

#[test]
fn repeated_cursor_and_descendant_bounds_fail_explicitly() {
    let mut harness = Harness::new(Path::new("/work/current"), None);
    harness.app.descendants_chain = Some(DescendantChain {
        generation: 1,
        root_id: "root".to_string(),
        pages: 0,
        cursors: HashSet::new(),
        ids: HashSet::new(),
    });
    harness
        .app
        .apply_descendants("root", 1, &json!({"data": [], "nextCursor": "same"}))
        .expect("apply first cursor");
    harness
        .app
        .apply_descendants("root", 1, &json!({"data": [], "nextCursor": "same"}))
        .expect("reject repeated cursor");
    assert!(
        harness
            .app
            .workspace
            .status_line
            .contains("repeated a pagination cursor")
    );

    harness.app.descendants_chain = Some(DescendantChain {
        generation: 1,
        root_id: "root".to_string(),
        pages: MAX_LIST_PAGES,
        cursors: HashSet::new(),
        ids: HashSet::new(),
    });
    harness
        .app
        .apply_descendants("root", 1, &json!({"data": []}))
        .expect("reject page overflow");
    assert!(
        harness
            .app
            .workspace
            .status_line
            .contains("exceeded 100 pages")
    );

    let ids = (0..=MAX_LIST_ITEMS)
        .map(|index| format!("item-{index}"))
        .collect::<HashSet<_>>();
    harness.app.descendants_chain = Some(DescendantChain {
        generation: 1,
        root_id: "root".to_string(),
        pages: 0,
        cursors: HashSet::new(),
        ids,
    });
    harness
        .app
        .apply_descendants(
            "root",
            1,
            &json!({"data": [{"id": "overflow", "parentThreadId": "root"}]}),
        )
        .expect("reject item overflow");
    assert!(
        harness
            .app
            .workspace
            .status_line
            .contains("exceeded 20000 unique threads")
    );
}

#[test]
fn recovery_selection_uses_saved_or_current_cwd_and_rejects_unsafe_saved_paths() {
    let cwd = Path::new("/work/current");
    let mut harness = Harness::new(cwd, None);
    let saved = harness._directory.0.join("saved");
    fs::create_dir(&saved).expect("create saved cwd");
    harness
        .app
        .select_session(Some(SessionSelection {
            id: "saved".to_string(),
            saved_cwd: saved.to_string_lossy().into_owned(),
            use_saved_cwd: true,
        }))
        .expect("select saved cwd");
    assert!(harness.requests().iter().any(|request| {
        request["method"] == "thread/resume" && request["params"]["cwd"] == json!(saved)
    }));

    let mut current = Harness::new(cwd, None);
    current
        .app
        .select_session(Some(SessionSelection {
            id: "current".to_string(),
            saved_cwd: "/old/path".to_string(),
            use_saved_cwd: false,
        }))
        .expect("select current cwd");
    assert!(current.requests().iter().any(|request| {
        request["method"] == "thread/resume" && request["params"]["cwd"] == json!(cwd)
    }));

    let trash = harness._directory.0.join("Trash/files/project");
    fs::create_dir_all(&trash).expect("create Trash cwd");
    for unsafe_path in [PathBuf::from("/definitely/missing"), trash] {
        let mut unsafe_harness = Harness::new(cwd, None);
        unsafe_harness
            .app
            .select_session(Some(SessionSelection {
                id: "unsafe".to_string(),
                saved_cwd: unsafe_path.to_string_lossy().into_owned(),
                use_saved_cwd: true,
            }))
            .expect("reject unsafe cwd");
        assert!(
            !unsafe_harness
                .methods()
                .iter()
                .any(|method| method == "thread/resume")
        );
        assert!(
            unsafe_harness
                .app
                .workspace
                .status_line
                .contains("current directory")
        );
    }
}

#[test]
fn selected_saved_cwd_is_used_when_resume_and_send_is_needed_later() {
    let cwd = Path::new("/work/current");
    let mut harness = Harness::new(cwd, None);
    let saved = harness._directory.0.join("saved");
    fs::create_dir(&saved).expect("create saved cwd");
    harness.app.active_session_cwd = saved.clone();
    harness.app.upsert_thread(&thread_value("root", &saved));
    harness.app.workspace.rebuild_tree(Some("root"));

    harness
        .app
        .submit(Submission {
            displayed_text: "continue".to_string(),
            input: vec![SubmissionInput::Text("continue".to_string())],
        })
        .expect("submit to resumed root");

    assert!(harness.requests().iter().any(|request| {
        request["method"] == "thread/resume" && request["params"]["cwd"] == json!(saved)
    }));
}
