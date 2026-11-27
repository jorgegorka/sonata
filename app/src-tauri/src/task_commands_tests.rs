//! Exercise the desktop IPC wrappers from a thread without a Tokio runtime.
use super::*;
use serde_json::{json, Value};
use sonata_core::model::{MessageStatus, NewBot};
use sonata_core::runtime::fake::FakeRuntime;
use sonata_core::work_items::WorkItem;
use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};

struct Harness {
    _tmp: tempfile::TempDir,
    paths: AppPaths,
    engine: Engine,
    runtime: Arc<FakeRuntime>,
    worker: Bot,
    _app: tauri::App<MockRuntime>,
    webview: tauri::WebviewWindow<MockRuntime>,
}

fn engine_at(paths: &AppPaths) -> (Engine, Arc<FakeRuntime>) {
    let runtime = FakeRuntime::new();
    let engine = Engine::new(
        Store::open(&paths.db_path()).unwrap(),
        runtime.clone(),
        EngineConfig {
            paths: paths.clone(),
            sbot_dir: paths.data_dir.join("bin"),
            fallback_lines: 200,
            turn_start_grace: None,
        },
    );
    (engine, runtime)
}

impl Harness {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::new(tmp.path().join("data"), tmp.path().join("bots"));
        paths.ensure().unwrap();
        let (engine, runtime) = engine_at(&paths);
        let worker = engine
            .create_bot(NewBot {
                display_name: "Worker".into(),
                color: "#456789".into(),
                provider: Provider::Claude,
                model: None,
                instructions: "Test worker".into(),
                skill_ids: vec![],
            })
            .unwrap();
        let app = mock_builder()
            .manage(engine.clone())
            .invoke_handler(tauri::generate_handler![create_work_item, start_work_item, set_paused])
            .build(mock_context(noop_assets()))
            .unwrap();
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
        Self { _tmp: tmp, paths, engine, runtime, worker, _app: app, webview }
    }

    fn invoke(&self, command: &str, body: Value) -> Result<Value, Value> {
        assert!(tokio::runtime::Handle::try_current().is_err(), "IPC caller must not enter Tokio");
        let webview = self.webview.clone();
        let command = command.to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        let caller = std::thread::spawn(move || {
            assert!(tokio::runtime::Handle::try_current().is_err());
            let response = tauri::test::get_ipc_response(
                &webview,
                tauri::webview::InvokeRequest {
                    cmd: command,
                    callback: tauri::ipc::CallbackFn(0),
                    error: tauri::ipc::CallbackFn(1),
                    url: "tauri://localhost".parse().unwrap(),
                    body: tauri::ipc::InvokeBody::Json(body),
                    headers: Default::default(),
                    invoke_key: tauri::test::INVOKE_KEY.to_string(),
                },
            )
            .map(|body| body.deserialize::<Value>().unwrap());
            let _ = tx.send(response);
        });
        let response =
            rx.recv_timeout(Duration::from_secs(10)).expect("desktop command must respond without panicking");
        caller.join().unwrap();
        response
    }

    fn input(&self, manual: bool) -> Value {
        json!({"instructions":"Execute the desktop task", "manual_start":manual,
            "executor":{"kind":"bot", "handle":self.worker.handle}})
    }

    fn create(&self, input: Value) -> WorkItem {
        serde_json::from_value(self.invoke("create_work_item", json!({"input":input, "projectId":"default"})).unwrap())
            .unwrap()
    }

    fn store(&self) -> Store {
        Store::open(&self.paths.db_path()).unwrap()
    }

    fn workflow(&self) -> Value {
        self.engine
            .create_workflow(
                serde_json::from_value(json!({
                    "name":"Review", "when_to_use":"Test", "def":{"version":1, "start":"review", "steps":[{
                        "id":"review", "name":"Review", "bot_id":self.worker.id, "instructions":"Review the request",
                        "arrows":[{"to":"done", "outcome":null}]
                    }]}
                }))
                .unwrap(),
            )
            .unwrap();
        json!({"kind":"workflow", "name":"Review"})
    }
}

fn eventually(mut check: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !check() {
        assert!(std::time::Instant::now() < deadline, "task did not reach the runtime");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn desktop_immediate_bot_and_workflow_tasks_deliver_the_request_and_attachment() {
    for workflow in [false, true] {
        let h = Harness::new();
        let file = h.paths.data_dir.join("report.txt");
        std::fs::write(&file, "Attached report").unwrap();
        let mut input = h.input(false);
        input["files"] = json!([file]);
        if workflow {
            input["executor"] = h.workflow();
        }
        let item = h.create(input);
        eventually(|| h.runtime.state().prompts.len() == 1);
        assert_eq!(item.state, "working");
        assert_eq!(item.workflow_run_id.is_some(), workflow);
        let messages = h.engine.session_messages(item.primary_session_id.as_ref().unwrap()).unwrap();
        assert_eq!(messages[0].status, MessageStatus::Delivered);
        assert_eq!(std::fs::read_to_string(&messages[0].attachments[0].path).unwrap(), "Attached report");
        assert!(h.runtime.state().prompts[0].1.contains("Execute the desktop task"));
    }
}

#[test]
fn desktop_manual_start_delivers_once_even_when_start_is_repeated() {
    for workflow in [false, true] {
        let h = Harness::new();
        let mut input = h.input(true);
        if workflow {
            input["executor"] = h.workflow();
        }
        let item = h.create(input);
        assert_eq!(item.state, "pending");
        assert!(h.runtime.state().starts.is_empty());
        for _ in 0..3 {
            h.invoke("start_work_item", json!({"taskId":item.id, "projectId":"default"})).unwrap();
        }
        eventually(|| h.runtime.state().prompts.len() == 1);
        assert_eq!(h.runtime.state().starts.len(), 1);
        assert_eq!(h.store().list_work_items().unwrap().len(), 1);
        assert_eq!(h.engine.sessions_of(&h.worker.id).unwrap().len(), 1);
    }
}

#[test]
fn desktop_subtask_of_active_parent_reaches_its_executor() {
    let h = Harness::new();
    let parent = h.create(h.input(false));
    eventually(|| h.runtime.state().prompts.len() == 1);
    let mut input = h.input(false);
    input["parent_id"] = json!(parent.id);
    input["instructions"] = json!("Execute the child task");
    let child = h.create(input);
    eventually(|| h.runtime.state().prompts.len() == 2);
    assert_eq!(child.parent_id, Some(parent.id));
    assert!(child.orchestration_id.is_some());
    assert!(h.runtime.state().prompts.iter().any(|(_, text)| text.contains("Execute the child task")));
}

#[test]
fn desktop_paused_tasks_wait_until_resumed() {
    let h = Harness::new();
    h.invoke("set_paused", json!({"paused":true})).unwrap();
    let item = h.create(h.input(false));
    assert_eq!(item.state, "pending");
    assert!(h.runtime.state().starts.is_empty());
    h.invoke("set_paused", json!({"paused":false})).unwrap();
    eventually(|| h.runtime.state().prompts.len() == 1);
    assert_eq!(h.store().list_work_items().unwrap()[0].id, item.id);
}

#[test]
fn desktop_invalid_requests_return_errors_and_allow_the_next_task() {
    let h = Harness::new();
    for project in ["default", "missing-project"] {
        let mut input = h.input(false);
        input["instructions"] = json!("");
        assert!(h.invoke("create_work_item", json!({"input":input, "projectId":project})).is_err());
    }
    let mut input = h.input(false);
    input["files"] = json!([h.paths.data_dir.join("missing.txt")]);
    assert!(h.invoke("create_work_item", json!({"input":input})).is_err());
    assert!(h.store().list_work_items().unwrap().is_empty());
    h.create(h.input(false));
    eventually(|| h.runtime.state().prompts.len() == 1);
}

#[test]
fn desktop_crash_queue_recovers_once_with_the_original_task_session_and_message() {
    let h = Harness::new();
    let item = h.create(h.input(true));
    // This is the committed state immediately before the crashing tokio::spawn.
    let routed = h.store().start_pending_work_item(&item.id, Some(&h.worker.id), None, &[]).unwrap().unwrap();
    assert_eq!(routed.message.status, MessageStatus::Queued);
    let (restarted, runtime) = engine_at(&h.paths);
    tauri::async_runtime::block_on(restarted.recover()).unwrap();
    tauri::async_runtime::block_on(restarted.recover()).unwrap();
    let stored = h.store().list_work_items().unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].id, item.id);
    assert_eq!(stored[0].primary_session_id.as_deref(), Some(routed.session.id.as_str()));
    let messages = restarted.session_messages(&routed.session.id).unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].id, routed.message.id);
    assert_eq!(messages[0].status, MessageStatus::Delivered);
    assert_eq!(runtime.state().prompts.len(), 1);
    assert_eq!(runtime.state().starts.len(), 1);
}
