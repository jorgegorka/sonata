use std::io::Read;
use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sonata_core::detect::{self, ProviderStatus};
use sonata_core::engine::{DialogAnswer, Engine, ExportSummary, ImportMode, ImportOutcome, ImportPreview};
use sonata_core::engine::{WorkflowFileEntry, WorkflowImportOutcome, WorkflowImportPreview, WorkflowImportRequest};
use sonata_core::model::*;
use sonata_core::CoreError;

#[derive(Debug, Deserialize)]
pub struct CreateBotInput {
    pub display_name: String,
    pub color: String,
    pub provider: Option<Provider>,
    pub model: Option<String>,
    pub instructions: String,
    #[serde(default)]
    pub skill_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CommandError {
    pub code: String,
    pub params: Value,
}

impl CommandError {
    pub fn new(code: &str, params: Value) -> Self {
        Self { code: code.to_string(), params }
    }

    pub fn internal(error: impl std::fmt::Display) -> Self {
        Self::new("internal", json!({ "error": error.to_string() }))
    }
}

impl From<CoreError> for CommandError {
    fn from(e: CoreError) -> Self {
        match e {
            CoreError::Project(error) => Self::new(error.code(), json!({})),
            CoreError::Refused(refusal) => Self::new(refusal.code(), refusal.params()),
            other => Self::internal(other),
        }
    }
}

pub async fn list_bots(engine: &Engine) -> Result<Vec<BotView>, CommandError> {
    Ok(engine.list_bots().await?)
}

fn new_bot(engine: &Engine, input: CreateBotInput, existing: Option<&str>) -> Result<NewBot, CommandError> {
    Ok(NewBot {
        display_name: input.display_name,
        color: input.color,
        provider: engine.resolve_bot_provider(input.provider, existing)?,
        model: input.model.filter(|m| !m.trim().is_empty()),
        instructions: input.instructions,
        skill_ids: input.skill_ids,
    })
}

pub fn create_bot(engine: &Engine, input: CreateBotInput) -> Result<Bot, CommandError> {
    let mut new = new_bot(engine, input, None)?;
    if sonata_core::bundled::find_checkout_paragraph(&new.instructions).is_none() {
        new.instructions.push_str(&format!(
            "\n\n{}",
            sonata_core::bundled::checkout_paragraph(engine.project_folder()?.as_deref())
        ));
    }
    Ok(engine.create_bot(new)?)
}

pub fn update_bot(engine: &Engine, bot_id: &str, input: CreateBotInput) -> Result<Bot, CommandError> {
    Ok(engine.update_bot(bot_id, new_bot(engine, input, Some(bot_id))?)?)
}

pub async fn delete_bot(
    engine: &Engine,
    bot_id: &str,
    trash: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), CommandError> {
    let folder = engine.delete_bot(bot_id).await?;
    if std::fs::symlink_metadata(&folder).is_ok() {
        trash(&folder).map_err(|error| {
            let handle = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            tracing::warn!(bot = %handle, folder = %folder.display(), error = %error, "could not move the bot's folder to the Trash");
            CommandError::new("trashFailed", json!({ "path": folder.to_string_lossy(), "error": error }))
        })?;
    }
    Ok(())
}

pub fn move_to_trash(path: &Path) -> Result<(), String> {
    #[allow(unused_mut)]
    let mut context = trash::TrashContext::default();
    #[cfg(target_os = "macos")]
    {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        context.set_delete_method(DeleteMethod::NsFileManager);
    }
    context.delete(path).map_err(|e| e.to_string())
}

fn finder(path: &Path) -> Command {
    let mut command = Command::new(if cfg!(target_os = "macos") { "/usr/bin/open" } else { "xdg-open" });
    crate::platform::host_command(&mut command);
    command.arg(path);
    command
}

pub async fn open_bot_folder(engine: &Engine, bot_id: &str) -> Result<(), CommandError> {
    run_open(finder(&engine.bot_folder(bot_id)?)).await
}

pub async fn open_bots_folder(engine: &Engine) -> Result<(), CommandError> {
    run_open(finder(&engine.paths().bots_root)).await
}

pub async fn open_logs_folder(engine: &Engine) -> Result<(), CommandError> {
    run_open(finder(&engine.paths().logs_dir())).await
}

pub async fn open_project_folder(engine: &Engine) -> Result<(), CommandError> {
    match engine.project_folder()? {
        Some(path) => run_open(finder(&path)).await,
        None => Ok(()),
    }
}

pub fn list_skills(engine: &Engine) -> Result<Vec<SkillView>, CommandError> {
    Ok(engine.list_skills()?)
}

pub fn create_skill(engine: &Engine, input: NewSkill) -> Result<Skill, CommandError> {
    Ok(engine.create_skill(input)?)
}

pub fn import_skill(engine: &Engine, path: &str) -> Result<Skill, CommandError> {
    Ok(engine.import_skill(Path::new(path))?)
}

pub fn export_config(engine: &Engine, path: &str) -> Result<ExportSummary, CommandError> {
    Ok(engine.write_config(Path::new(path))?)
}

pub fn export_workflow_config(engine: &Engine, workflow_id: &str, path: &str) -> Result<ExportSummary, CommandError> {
    Ok(engine.write_workflow_config(workflow_id, Path::new(path))?)
}

pub fn workflows_in_config(engine: &Engine, path: &str) -> Result<Vec<WorkflowFileEntry>, CommandError> {
    Ok(engine.workflows_in_config(Path::new(path))?)
}

pub fn preview_workflow_import(
    engine: &Engine,
    path: &str,
    request: &WorkflowImportRequest,
) -> Result<WorkflowImportPreview, CommandError> {
    Ok(engine.preview_workflow_import(Path::new(path), request)?)
}

pub fn import_workflow_config(
    engine: &Engine,
    path: &str,
    request: &WorkflowImportRequest,
    token: &str,
) -> Result<WorkflowImportOutcome, CommandError> {
    Ok(engine.import_workflow_config(Path::new(path), request, token)?)
}

pub fn preview_config_import(engine: &Engine, path: &str) -> Result<ImportPreview, CommandError> {
    Ok(engine.preview_config_import(Path::new(path))?)
}

pub fn import_config(
    engine: &Engine,
    path: &str,
    mode: ImportMode,
    token: &str,
) -> Result<ImportOutcome, CommandError> {
    Ok(engine.import_config(Path::new(path), mode, token)?)
}

pub fn get_skill_body(engine: &Engine, skill_id: &str) -> Result<String, CommandError> {
    Ok(engine.skill_body(skill_id)?)
}

pub fn update_skill(engine: &Engine, skill_id: &str, input: NewSkill) -> Result<Skill, CommandError> {
    Ok(engine.update_skill(skill_id, input)?)
}

pub fn delete_skill(
    engine: &Engine,
    skill_id: &str,
    trash: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), CommandError> {
    let folder = engine.delete_skill(skill_id)?;
    if std::fs::symlink_metadata(&folder).is_ok() {
        trash(&folder).map_err(|error| {
            let slug = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            tracing::warn!(skill = %slug, folder = %folder.display(), error = %error, "could not move the skill's folder to the Trash");
            CommandError::new("skillTrashFailed", json!({ "path": folder.to_string_lossy(), "error": error }))
        })?;
    }
    Ok(())
}

pub async fn open_skills_folder(engine: &Engine) -> Result<(), CommandError> {
    run_open(finder(&engine.skills_dir())).await
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AppSettings {
    pub hop_limit: i64,
    pub bots_root: String,
    pub logs_dir: String,
    pub bots_exist: bool,
    pub setup_done: bool,
    pub language: String,
    pub language_resolved: String,
    pub language_system: String,
    pub project_folder: Option<String>,
}

pub fn get_settings(engine: &Engine) -> Result<AppSettings, CommandError> {
    let paths = engine.paths();
    Ok(AppSettings {
        hop_limit: engine.hop_limit()?,
        bots_root: paths.bots_root.to_string_lossy().into_owned(),
        logs_dir: paths.logs_dir().to_string_lossy().into_owned(),
        bots_exist: engine.has_bots()?,
        setup_done: engine.setup_done()?,
        language: engine.language_choice()?,
        language_resolved: engine.language()?,
        language_system: engine.system_language(),
        project_folder: engine.project_folder()?.map(|path| path.to_string_lossy().into_owned()),
    })
}

pub fn set_hop_limit(engine: &Engine, limit: i64) -> Result<(), CommandError> {
    Ok(engine.set_hop_limit(limit)?)
}

pub fn set_language(engine: &Engine, choice: &str) -> Result<String, CommandError> {
    engine.set_language(choice)?;
    Ok(engine.language()?)
}

pub fn set_bots_root(engine: &Engine, path: &str) -> Result<(), CommandError> {
    Ok(engine.set_bots_root(Path::new(path))?)
}

pub fn set_project_folder(engine: &Engine, path: &str) -> Result<Vec<String>, CommandError> {
    Ok(engine.set_project_folder(Path::new(path))?)
}

pub fn finish_setup(engine: &Engine, project_folder: Option<&str>) -> Result<(), CommandError> {
    let checkout = project_folder.map(str::trim).filter(|path| !path.is_empty()).map(Path::new);
    Ok(engine.finish_setup(checkout)?)
}

pub async fn detect_providers(search_path: &str) -> Vec<ProviderStatus> {
    let (claude, codex, grok, opencode) = tokio::join!(
        detect::detect(Provider::Claude, search_path),
        detect::detect(Provider::Codex, search_path),
        detect::detect(Provider::Grok, search_path),
        detect::detect(Provider::Opencode, search_path)
    );
    vec![claude, codex, grok, opencode]
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HerdrStatus {
    pub path: String,
    pub version: Option<String>,
    pub protocol: Option<u32>,
    pub error: Option<String>,
}

pub async fn herdr_status(engine: &Engine, herdr: &Path) -> HerdrStatus {
    let path = herdr.to_string_lossy().into_owned();
    match engine.runtime_health().await {
        Ok(health) => HerdrStatus { path, version: Some(health.version), protocol: Some(health.protocol), error: None },
        Err(e) => HerdrStatus { path, version: None, protocol: None, error: Some(e.to_string()) },
    }
}

pub async fn list_sessions(engine: &Engine) -> Result<Vec<SessionView>, CommandError> {
    Ok(engine.all_sessions().await?)
}

pub fn get_session_messages(engine: &Engine, session_id: &str) -> Result<Vec<Message>, CommandError> {
    Ok(engine.session_messages(session_id)?)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Sent {
    pub session: SessionView,
    pub message: Message,
}

pub async fn send_message(
    engine: &Engine,
    bot_id: &str,
    session_id: Option<&str>,
    body: &str,
    files: &[String],
) -> Result<Sent, CommandError> {
    let (session, message) = match session_id {
        Some(id) => {
            let message = engine.queue_user_message_to_session(id, body, files)?;
            (engine.find_session_view(id).await?, message)
        }
        None => {
            let (session, message) = engine.start_session_with_message(bot_id, body, files)?;
            (engine.find_session_view(&session.id).await?, message)
        }
    };
    tokio::spawn(engine.pump_session(&session.session.id));
    Ok(Sent { session, message })
}

pub async fn request_workflow(engine: &Engine, body: &str) -> Result<Sent, CommandError> {
    let (session, message) = engine.start_workflow_request(body)?;
    let session = engine.find_session_view(&session.id).await?;
    tokio::spawn(engine.pump_session(&session.session.id));
    Ok(Sent { session, message })
}

pub async fn cancel_message(engine: &Engine, message_id: &str) -> Result<Message, CommandError> {
    Ok(engine.cancel_message(message_id).await?)
}

pub async fn retry_message(engine: &Engine, message_id: &str) -> Result<Message, CommandError> {
    let message = engine.retry_message(message_id)?;
    match &message.recipient_session {
        Some(session_id) => tokio::spawn(engine.pump_session(session_id)),
        None => tokio::spawn(engine.pump(&message.recipient)),
    };
    Ok(message)
}

pub async fn stop_turn(engine: &Engine, session_id: &str) -> Result<(), CommandError> {
    Ok(engine.stop_turn(session_id).await?)
}

pub async fn answer_dialog(engine: &Engine, session_id: &str, answer: &str) -> Result<(), CommandError> {
    let answer = match answer {
        "continue" => DialogAnswer::Continue,
        "cancel" => DialogAnswer::Cancel,
        other => return Err(CommandError::internal(format!("unknown answer: {other}"))),
    };
    Ok(engine.answer_dialog(session_id, answer).await?)
}

pub async fn session_snapshot(engine: &Engine, session_id: &str) -> Result<Option<String>, CommandError> {
    Ok(engine.snapshot(session_id).await?)
}

pub async fn session_terminal(engine: &Engine, session_id: &str) -> Result<Option<String>, CommandError> {
    Ok(engine.terminal(session_id).await?)
}

pub async fn restart_session(engine: &Engine, session_id: &str) -> Result<(), CommandError> {
    Ok(engine.restart_session(session_id).await?)
}

pub async fn stop_session(engine: &Engine, session_id: &str) -> Result<(), CommandError> {
    Ok(engine.stop_session(session_id).await?)
}

pub async fn remove_session(
    engine: &Engine,
    session_id: &str,
    mut trash: impl FnMut(&Path) -> Result<(), String>,
) -> Result<Vec<String>, CommandError> {
    let removed = engine.remove_session(session_id).await?;
    for folder in &removed.drafts {
        if let Err(error) = trash(folder) {
            let bot = folder.ancestors().nth(2).and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned());
            tracing::warn!(bot = %bot.unwrap_or_default(), folder = %folder.display(), error = %error, "could not move the removed session's drafts to the Trash");
        }
    }
    Ok(removed.sessions.into_iter().map(|s| s.session.id).collect())
}

pub async fn rename_session(engine: &Engine, session_id: &str, name: &str) -> Result<SessionView, CommandError> {
    Ok(engine.rename_session(session_id, name).await?)
}

pub async fn archive_session(engine: &Engine, session_id: &str) -> Result<Vec<String>, CommandError> {
    Ok(engine.archive_session(session_id).await?.into_iter().map(|s| s.id).collect())
}

pub fn mark_session_seen(engine: &Engine, session_id: &str) -> Result<(), CommandError> {
    Ok(engine.mark_session_seen(session_id)?)
}

pub async fn session_activity(engine: &Engine, session_ids: &[String]) -> Result<Vec<SessionActivity>, CommandError> {
    Ok(engine.session_activity(session_ids).await?)
}

pub async fn session_glances(engine: &Engine, session_ids: &[String]) -> Vec<Glance> {
    engine.session_glances(session_ids).await
}

pub async fn restore_session(engine: &Engine, session_id: &str) -> Result<Vec<String>, CommandError> {
    Ok(engine.restore_session(session_id).await?.into_iter().map(|s| s.id).collect())
}

pub fn list_tasks(engine: &Engine) -> Result<Vec<Task>, CommandError> {
    Ok(engine.list_tasks()?)
}

pub fn create_task(engine: &Engine, input: NewTask) -> Result<Task, CommandError> {
    Ok(engine.create_task(input)?)
}

pub fn update_task(engine: &Engine, task_id: &str, input: NewTask) -> Result<Task, CommandError> {
    Ok(engine.update_task(task_id, input)?)
}

pub fn set_task_enabled(engine: &Engine, task_id: &str, enabled: bool) -> Result<Task, CommandError> {
    Ok(engine.set_task_enabled(task_id, enabled)?)
}

pub fn delete_task(engine: &Engine, task_id: &str) -> Result<(), CommandError> {
    engine.delete_task(task_id)?;
    Ok(())
}

pub async fn run_task_now(engine: &Engine, task_id: &str) -> Result<SessionView, CommandError> {
    let session = engine.run_task_now(task_id)?;
    Ok(engine.find_session_view(&session.id).await?)
}

pub fn list_workflows(engine: &Engine) -> Result<Vec<Workflow>, CommandError> {
    Ok(engine.list_workflows()?)
}

pub fn create_workflow(engine: &Engine, input: NewWorkflow) -> Result<Workflow, CommandError> {
    Ok(engine.create_workflow(input)?)
}

pub fn update_workflow(engine: &Engine, workflow_id: &str, input: NewWorkflow) -> Result<Workflow, CommandError> {
    Ok(engine.update_workflow(workflow_id, input)?)
}

pub fn delete_workflow(engine: &Engine, workflow_id: &str) -> Result<(), CommandError> {
    engine.delete_workflow(workflow_id)?;
    Ok(())
}

pub async fn start_workflow(
    engine: &Engine,
    workflow_id: &str,
    body: &str,
    files: &[String],
) -> Result<SessionView, CommandError> {
    let session = engine.start_workflow(workflow_id, body, files)?;
    Ok(engine.find_session_view(&session.id).await?)
}

pub fn get_workflow_run(engine: &Engine, run_id: &str) -> Result<RunView, CommandError> {
    Ok(engine.get_workflow_run(run_id)?)
}

pub fn another_round(engine: &Engine, run_id: &str) -> Result<(), CommandError> {
    Ok(engine.another_round(run_id)?)
}

pub fn pick_outcome(engine: &Engine, run_id: &str, outcome: Option<String>) -> Result<(), CommandError> {
    Ok(engine.pick_outcome(run_id, outcome)?)
}

pub fn remind_step(engine: &Engine, run_id: &str) -> Result<(), CommandError> {
    Ok(engine.remind_step(run_id)?)
}

pub async fn stop_run(engine: &Engine, run_id: &str) -> Result<(), CommandError> {
    Ok(engine.stop_run(run_id).await?)
}

pub fn get_paused(engine: &Engine) -> Result<bool, CommandError> {
    Ok(engine.paused()?)
}

pub async fn set_paused(engine: &Engine, paused: bool) -> Result<(), CommandError> {
    Ok(engine.set_paused(paused).await?)
}

pub fn usage_limits(engine: &Engine) -> Result<Vec<UsageLimitView>, CommandError> {
    Ok(engine.usage_limits()?)
}

pub async fn try_usage_limit_now(engine: &Engine, provider: Provider) -> Result<(), CommandError> {
    Ok(engine.try_usage_limit_now(provider).await?)
}

pub fn list_bot_threads(engine: &Engine) -> Result<Vec<ThreadSummary>, CommandError> {
    Ok(engine.bot_threads()?)
}

pub fn get_thread_messages(engine: &Engine, thread_id: &str) -> Result<Vec<Message>, CommandError> {
    Ok(engine.thread_messages(thread_id)?)
}

fn is_finder_alias(path: &Path) -> bool {
    let mut start = Vec::with_capacity(16);
    std::fs::File::open(path).and_then(|f| f.take(16).read_to_end(&mut start)).is_ok() && start.starts_with(b"book")
}

pub fn attachment_command(
    engine: &Engine,
    message_id: &str,
    index: usize,
    reveal: bool,
) -> Result<Command, CommandError> {
    let path = engine.attachment_path(message_id, index)?;
    if !path.exists() {
        return Err(CommandError::new("fileGone", json!({ "path": path.to_string_lossy() })));
    }
    if reveal {
        #[cfg(target_os = "macos")]
        {
            let mut command = Command::new("/usr/bin/open");
            command.arg("-R").arg(path);
            return Ok(command);
        }
        #[cfg(not(target_os = "macos"))]
        return Ok(finder(path.parent().unwrap_or(&path)));
    }
    let refused = || CommandError::new("opensPrograms", json!({}));
    let target = std::fs::canonicalize(&path).map_err(|_| refused())?;
    if sonata_core::attachments::runs_programs(&target) || is_finder_alias(&target) {
        return Err(refused());
    }
    Ok(finder(&target))
}

pub async fn run_open(mut command: Command) -> Result<(), CommandError> {
    let failed = |error: String| CommandError::new("openFailed", json!({ "error": error }));
    let status = tokio::task::spawn_blocking(move || command.status())
        .await
        .map_err(CommandError::internal)?
        .map_err(|e| failed(e.to_string()))?;
    if status.success() {
        Ok(())
    } else {
        Err(failed(status.to_string()))
    }
}

pub async fn open_attachment(
    engine: &Engine,
    message_id: &str,
    index: usize,
    reveal: bool,
) -> Result<(), CommandError> {
    run_open(attachment_command(engine, message_id, index, reveal)?).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use sonata_core::engine::EngineConfig;
    use sonata_core::paths::AppPaths;
    use sonata_core::runtime::fake::FakeRuntime;
    use sonata_core::store::Store;
    use std::sync::Arc;
    use std::time::Duration;

    fn setup() -> (Engine, Arc<FakeRuntime>, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::new(tmp.path().join("data"), tmp.path().join("bots"));
        paths.ensure().unwrap();
        let rt = FakeRuntime::new();
        let engine = Engine::new(
            Store::open_in_memory().unwrap(),
            rt.clone(),
            EngineConfig { paths, sbot_dir: tmp.path().into(), fallback_lines: 200, turn_start_grace: None },
        );
        (engine, rt, tmp)
    }

    fn input(name: &str) -> CreateBotInput {
        CreateBotInput {
            display_name: name.into(),
            color: "#3b82f6".into(),
            provider: Some(Provider::Claude),
            model: None,
            instructions: String::new(),
            skill_ids: vec![],
        }
    }

    fn new_task(bot_id: &str, name: &str, every_days: u32) -> NewTask {
        NewTask {
            name: name.into(),
            bot_id: bot_id.into(),
            prompt: "Summarize the week".into(),
            every_days,
            at: "09:00".into(),
            starts_on: None,
        }
    }

    #[tokio::test]
    async fn config_moves_between_two_installs_through_the_commands() {
        let (first, _rt1, tmp) = setup();
        let (second, _rt2, _tmp2) = setup();
        create_bot(&first, input("Writer")).unwrap();
        let file = tmp.path().join("config.yaml");
        export_config(&first, file.to_str().unwrap()).unwrap();

        let preview = preview_config_import(&second, file.to_str().unwrap()).unwrap();
        let writer = preview.bots.iter().find(|item| item.name == "writer").unwrap();
        assert_eq!(writer.status, sonata_core::engine::ItemStatus::New);

        let outcome = import_config(&second, file.to_str().unwrap(), ImportMode::AddNew, &preview.token).unwrap();
        assert!(matches!(outcome, ImportOutcome::Done { .. }));
        assert!(list_bots(&second).await.unwrap().iter().any(|bot| bot.bot.handle == "writer"));
    }

    #[tokio::test]
    async fn a_workflow_and_its_bot_move_through_the_workflow_commands() {
        let (first, _rt1, tmp) = setup();
        let (second, _rt2, _tmp2) = setup();
        let bot = create_bot(&first, input("Writer")).unwrap();
        let workflow = create_workflow(&first, new_workflow("Write", &bot.id)).unwrap();
        let path = tmp.path().join("workflow.yaml");
        let path = path.to_str().unwrap();
        assert_eq!(
            export_workflow_config(&first, &workflow.id, path).unwrap(),
            ExportSummary { bots: 1, skills: 0, workflows: 1 }
        );
        let entries = workflows_in_config(&second, path).unwrap();
        assert_eq!(entries[0].name, "Write");
        let request = WorkflowImportRequest { workflow: "Write".into(), ..Default::default() };
        let preview = preview_workflow_import(&second, path, &request).unwrap();
        assert!(preview.can_import);
        let outcome = import_workflow_config(&second, path, &request, &preview.token).unwrap();
        assert!(matches!(outcome, WorkflowImportOutcome::Done { .. }));
        assert_eq!(list_workflows(&second).unwrap().len(), 1);
        assert_eq!(list_bots(&second).await.unwrap()[0].bot.handle, "writer");
    }

    #[tokio::test]
    async fn task_commands_round_trip() {
        let (engine, _rt, _tmp) = setup();
        let bot = create_bot(&engine, input("Researcher")).unwrap();
        let task = create_task(&engine, new_task(&bot.id, "Weekly", 7)).unwrap();
        assert_eq!(list_tasks(&engine).unwrap().len(), 1);

        let edited = update_task(&engine, &task.id, new_task(&bot.id, "Weekly digest", 3)).unwrap();
        assert_eq!((edited.name.as_str(), edited.every_days), ("Weekly digest", 3));

        let off = set_task_enabled(&engine, &task.id, false).unwrap();
        assert!(!off.enabled);

        let run = run_task_now(&engine, &task.id).await.unwrap();
        assert_eq!(run.session.bot_id, bot.id);
        assert!(!list_tasks(&engine).unwrap()[0].enabled);

        delete_task(&engine, &task.id).unwrap();
        assert!(list_tasks(&engine).unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_bad_task_is_a_coded_error() {
        let (engine, _rt, _tmp) = setup();
        let bot = create_bot(&engine, input("Researcher")).unwrap();
        let err = create_task(&engine, new_task(&bot.id, "Weekly", 0)).unwrap_err();
        assert_eq!(err.code, "taskEveryRange");
        assert_eq!(err.params, json!({ "min": "1", "max": "365" }));
    }

    fn new_workflow(name: &str, bot_id: &str) -> NewWorkflow {
        NewWorkflow {
            name: name.into(),
            when_to_use: String::new(),
            def: WorkflowDef {
                version: WORKFLOW_FORMAT,
                start: Some("a".into()),
                steps: vec![Step {
                    id: "a".into(),
                    name: "Write".into(),
                    bot_id: bot_id.into(),
                    instructions: "Write it".into(),
                    arrows: vec![Arrow { to: DONE.into(), outcome: None }],
                    pos: None,
                }],
                start_pos: None,
                done_pos: None,
            },
        }
    }

    #[tokio::test]
    async fn workflow_commands_round_trip() {
        let (engine, _rt, _tmp) = setup();
        let bot = create_bot(&engine, input("Writer")).unwrap();
        let wf = create_workflow(&engine, new_workflow("Draft", &bot.id)).unwrap();
        assert_eq!(list_workflows(&engine).unwrap().len(), 1);
        let edited = update_workflow(&engine, &wf.id, new_workflow("Draft it", &bot.id)).unwrap();
        assert_eq!(edited.name, "Draft it");

        let session = start_workflow(&engine, &wf.id, "Write a poem", &[]).await.unwrap();
        assert_eq!(session.session.bot_id, bot.id);
        let activity = engine.session_activity(std::slice::from_ref(&session.session.id)).await.unwrap();
        let run_id = activity[0].workflow.as_ref().expect("the session is in a run").run_id.clone();
        let view = get_workflow_run(&engine, &run_id).unwrap();
        assert_eq!(view.run.request, "Write a poem");

        stop_run(&engine, &run_id).await.unwrap();
        assert_eq!(get_workflow_run(&engine, &run_id).unwrap().run.status, RunStatus::Stopped);
        delete_workflow(&engine, &wf.id).unwrap();
        assert!(list_workflows(&engine).unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_bad_workflow_is_a_coded_error() {
        let (engine, _rt, _tmp) = setup();
        let err = create_workflow(&engine, new_workflow("Draft", "no-such-bot")).unwrap_err();
        assert_eq!(err.code, "workflowStepNoBot");
    }

    #[tokio::test]
    async fn created_bots_are_listed_offline() {
        let (engine, _rt, _tmp) = setup();
        let bot = create_bot(&engine, input("Researcher")).unwrap();
        let bots = list_bots(&engine).await.unwrap();
        assert_eq!(bots.len(), 1);
        assert_eq!(bots[0].bot.id, bot.id);
        assert_eq!(bots[0].status, BotStatus::Offline);
    }

    #[tokio::test]
    async fn send_message_without_a_session_starts_one_and_delivers_in_the_background() {
        let (engine, rt, _tmp) = setup();
        let bot = create_bot(&engine, input("Researcher")).unwrap();
        let sent = send_message(&engine, &bot.id, None, "hi\nmore", &[]).await.unwrap();
        assert_eq!(sent.message.status, MessageStatus::Queued);
        assert_eq!((sent.session.session.bot_id.as_str(), sent.session.session.number), (bot.id.as_str(), 1));
        assert_eq!(sent.session.session.title.as_deref(), Some("hi"));
        assert_eq!(sent.message.recipient_session.as_deref(), Some(sent.session.session.id.as_str()));
        wait_for_prompts(&rt, 1).await;
        let chat = get_session_messages(&engine, &sent.session.session.id).unwrap();
        assert_eq!(chat.len(), 1);
        assert_eq!(chat[0].status, MessageStatus::Delivered);
        assert_eq!(list_sessions(&engine).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn send_message_with_a_session_continues_it_and_a_second_blank_chat_is_a_second_session() {
        let (engine, rt, _tmp) = setup();
        let bot = create_bot(&engine, input("Researcher")).unwrap();
        let first = send_message(&engine, &bot.id, None, "one", &[]).await.unwrap();
        wait_for_prompts(&rt, 1).await;
        let id = first.session.session.id.clone();
        let again = send_message(&engine, &bot.id, Some(&id), "two", &[]).await.unwrap();
        assert_eq!(again.session.session.id, id);
        assert_eq!(get_session_messages(&engine, &id).unwrap().len(), 2);
        let other = send_message(&engine, &bot.id, None, "three", &[]).await.unwrap();
        assert_ne!(other.session.session.id, id);
        assert_eq!(other.session.session.number, 2);
        let mut ids: Vec<String> = list_sessions(&engine).await.unwrap().into_iter().map(|s| s.session.id).collect();
        ids.sort();
        let mut expected = vec![id.clone(), other.session.session.id.clone()];
        expected.sort();
        assert_eq!(ids, expected);
        assert_eq!(get_session_messages(&engine, &other.session.session.id).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_workflow_request_opens_a_master_session_and_needs_master() {
        let (engine, rt, _tmp) = setup();
        assert_eq!(request_workflow(&engine, "An SEO workflow").await.unwrap_err().code, "masterMissing");
        engine.finish_setup(None).unwrap();
        let master = engine.master_id().unwrap().unwrap();
        let sent = request_workflow(
            &engine,
            "An SEO workflow
with tickets",
        )
        .await
        .unwrap();
        assert_eq!(sent.session.session.bot_id, master);
        assert_eq!(sent.session.session.title.as_deref(), Some("An SEO workflow"));
        assert!(sent.message.workflow_request);
        wait_for_prompts(&rt, 1).await;
        let chat = get_session_messages(&engine, &sent.session.session.id).unwrap();
        assert_eq!((chat.len(), chat[0].status, chat[0].workflow_request), (1, MessageStatus::Delivered, true));
    }

    #[tokio::test]
    async fn the_session_controls_act_on_the_session_named() {
        let (engine, rt, _tmp) = setup();
        let bot = create_bot(&engine, input("Researcher")).unwrap();
        let sent = send_message(&engine, &bot.id, None, "hi", &[]).await.unwrap();
        let id = sent.session.session.id;
        wait_for_prompts(&rt, 1).await;
        rt.state().screen = "Working…".into();
        assert_eq!(session_snapshot(&engine, &id).await.unwrap().as_deref(), Some("Working…"));
        stop_turn(&engine, &id).await.unwrap();
        assert_eq!(rt.state().keys, vec![("w1:p1".to_string(), vec!["esc".to_string()])]);
        stop_session(&engine, &id).await.unwrap();
        assert_eq!(session_snapshot(&engine, &id).await.unwrap(), None, "stopped");
        assert_eq!(remove_session(&engine, &id, |_| Ok(())).await.unwrap(), vec![id.clone()]);
        assert!(list_sessions(&engine).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn errors_carry_a_code_the_ui_translates() {
        let (engine, _rt, _tmp) = setup();
        assert_eq!(send_message(&engine, "missing", None, "hi", &[]).await.unwrap_err().code, "botNotFound");
        assert_eq!(create_bot(&engine, input("  ")).unwrap_err().code, "nameRequired");
        let err = answer_dialog(&engine, "session", "maybe").await.unwrap_err();
        assert_eq!(err.code, "internal");
        assert!(err.params["error"].as_str().unwrap().contains("unknown answer"), "{err:?}");
    }

    #[tokio::test]
    async fn an_unknown_session_is_refused_by_every_session_command() {
        let (engine, _rt, _tmp) = setup();
        let bot = create_bot(&engine, input("Researcher")).unwrap();
        assert_eq!(get_session_messages(&engine, "nope").unwrap_err().code, "sessionNotFound");
        assert_eq!(send_message(&engine, &bot.id, Some("nope"), "hi", &[]).await.unwrap_err().code, "sessionNotFound");
        assert_eq!(restart_session(&engine, "nope").await.unwrap_err().code, "sessionNotFound");
        assert_eq!(remove_session(&engine, "nope", |_| Ok(())).await.unwrap_err().code, "sessionNotFound");
        assert_eq!(stop_session(&engine, "nope").await.unwrap_err().code, "sessionNotFound");
        assert_eq!(rename_session(&engine, "nope", "Plan").await.unwrap_err().code, "sessionNotFound");
        assert_eq!(archive_session(&engine, "nope").await.unwrap_err().code, "sessionNotFound");
        assert_eq!(mark_session_seen(&engine, "nope").unwrap_err().code, "sessionNotFound");
        assert_eq!(restore_session(&engine, "nope").await.unwrap_err().code, "sessionNotFound");
    }

    #[tokio::test]
    async fn session_activity_lists_the_asked_sessions() {
        let (engine, _rt, _tmp) = setup();
        let bot = create_bot(&engine, input("Researcher")).unwrap();
        let (session, _) = engine.start_session_with_message(&bot.id, "Fix it", &[]).unwrap();
        let asked = [session.id.clone(), "gone".to_string()];
        let activity = session_activity(&engine, &asked).await.unwrap();
        assert_eq!(activity.len(), 1);
        assert_eq!(activity[0].session_id, session.id);
    }

    #[tokio::test]
    async fn session_glances_leave_out_sessions_with_no_cli() {
        let (engine, _rt, _tmp) = setup();
        assert!(session_glances(&engine, &["gone".to_string()]).await.is_empty());
    }

    #[tokio::test]
    async fn archive_and_restore_round_trip_and_an_archived_session_refuses_messages() {
        let (engine, rt, _tmp) = setup();
        let bot = create_bot(&engine, input("Researcher")).unwrap();
        let id = send_message(&engine, &bot.id, None, "hi", &[]).await.unwrap().session.session.id;
        wait_for_prompts(&rt, 1).await;

        assert_eq!(archive_session(&engine, &id).await.unwrap(), vec![id.clone()]);
        let refused = send_message(&engine, &bot.id, Some(&id), "more", &[]).await.unwrap_err();
        assert_eq!(refused.code, "sessionArchived");
        let listed = list_sessions(&engine).await.unwrap();
        assert!(listed[0].session.archived_at.is_some(), "archived sessions are listed, marked");

        assert_eq!(restore_session(&engine, &id).await.unwrap(), vec![id.clone()]);
        send_message(&engine, &bot.id, Some(&id), "more", &[]).await.unwrap();
        wait_for_prompts(&rt, 2).await;
    }

    #[tokio::test]
    async fn rename_session_returns_the_session_with_its_new_title() {
        let (engine, _rt, _tmp) = setup();
        let bot = create_bot(&engine, input("Researcher")).unwrap();
        let id = send_message(&engine, &bot.id, None, "hi", &[]).await.unwrap().session.session.id;
        let view = rename_session(&engine, &id, "Release notes").await.unwrap();
        let json = serde_json::to_value(&view).unwrap();
        assert_eq!((json["id"].as_str(), json["title"].as_str()), (Some(id.as_str()), Some("Release notes")));
        assert!(json["archived_at"].is_null(), "{json}");
    }

    async fn wait_for_prompts(rt: &FakeRuntime, n: usize) {
        for _ in 0..100 {
            if rt.state().prompts.len() >= n {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("expected {n} prompts, got {}", rt.state().prompts.len());
    }

    #[tokio::test]
    async fn send_message_copies_files_and_opens_them_only_through_their_message() {
        let (engine, rt, tmp) = setup();
        let bot = create_bot(&engine, input("Researcher")).unwrap();
        let file = tmp.path().join("notes.md");
        std::fs::write(&file, "n").unwrap();
        let msg =
            send_message(&engine, &bot.id, None, "", &[file.to_string_lossy().into_owned()]).await.unwrap().message;
        wait_for_prompts(&rt, 1).await;
        let copy = &msg.attachments[0].path;
        let reveal = attachment_command(&engine, &msg.id, 0, true).unwrap();
        #[cfg(target_os = "macos")]
        {
            assert_eq!(reveal.get_program(), "/usr/bin/open");
            assert_eq!(reveal.get_args().collect::<Vec<_>>(), vec!["-R", copy.as_str()]);
        }
        #[cfg(target_os = "linux")]
        {
            assert_eq!(reveal.get_program(), "xdg-open");
            assert_eq!(reveal.get_args().collect::<Vec<_>>(), vec![Path::new(copy).parent().unwrap().as_os_str()]);
        }
        let open = attachment_command(&engine, &msg.id, 0, false).unwrap();
        let canonical = std::fs::canonicalize(copy).unwrap();
        #[cfg(target_os = "macos")]
        assert_ne!(canonical.to_string_lossy(), copy.as_str(), "the test needs a path with a link in it");
        assert_eq!(open.get_args().collect::<Vec<_>>(), vec![canonical.as_os_str()]);
        assert!(attachment_command(&engine, &msg.id, 1, false).is_err());
    }

    async fn message_with_file(name: &str, bytes: &[u8]) -> (Engine, Message, tempfile::TempDir) {
        let (engine, rt, tmp) = setup();
        let bot = create_bot(&engine, input("Researcher")).unwrap();
        let file = tmp.path().join(name);
        std::fs::write(&file, bytes).unwrap();
        let msg =
            send_message(&engine, &bot.id, None, "", &[file.to_string_lossy().into_owned()]).await.unwrap().message;
        wait_for_prompts(&rt, 1).await;
        (engine, msg, tmp)
    }

    #[tokio::test]
    async fn open_follows_a_link_and_refuses_a_target_that_runs_programs() {
        let (engine, msg, tmp) = message_with_file("report.md", b"notes").await;
        let payload = tmp.path().join("payload.pkg");
        std::fs::write(&payload, "x").unwrap();
        let copy = &msg.attachments[0].path;
        std::fs::remove_file(copy).unwrap();
        std::os::unix::fs::symlink(&payload, copy).unwrap();
        let err = attachment_command(&engine, &msg.id, 0, false).unwrap_err();
        assert_eq!(err.code, "opensPrograms");
        let reveal = attachment_command(&engine, &msg.id, 0, true).unwrap();
        #[cfg(target_os = "macos")]
        assert_eq!(reveal.get_args().collect::<Vec<_>>(), vec!["-R", copy.as_str()], "Show in Finder still works");
        #[cfg(target_os = "linux")]
        assert_eq!(
            reveal.get_args().collect::<Vec<_>>(),
            vec![Path::new(copy).parent().unwrap().as_os_str()],
            "The containing folder can still be shown safely"
        );
    }

    #[tokio::test]
    async fn a_file_that_is_gone_is_named_in_the_error() {
        let (engine, msg, _tmp) = message_with_file("report.md", b"notes").await;
        let copy = msg.attachments[0].path.clone();
        std::fs::remove_file(&copy).unwrap();
        let err = attachment_command(&engine, &msg.id, 0, true).unwrap_err();
        assert_eq!((err.code.as_str(), err.params["path"].as_str()), ("fileGone", Some(copy.as_str())));
    }

    #[tokio::test]
    async fn open_refuses_a_finder_alias() {
        let (engine, msg, _tmp) = message_with_file("report.md", b"book\0\0\0\0mark\0\0\0\0\x38\0\0\0").await;
        let err = attachment_command(&engine, &msg.id, 0, false).unwrap_err();
        assert_eq!(err.code, "opensPrograms");
        assert!(attachment_command(&engine, &msg.id, 0, true).is_ok(), "Show in Finder still works");
    }

    #[tokio::test]
    async fn files_that_run_programs_are_only_shown_in_finder() {
        let (engine, rt, tmp) = setup();
        let bot = create_bot(&engine, input("Researcher")).unwrap();
        let script = tmp.path().join("setup.command");
        std::fs::write(&script, "echo hi").unwrap();
        let msg =
            send_message(&engine, &bot.id, None, "", &[script.to_string_lossy().into_owned()]).await.unwrap().message;
        wait_for_prompts(&rt, 1).await;
        assert!(attachment_command(&engine, &msg.id, 0, true).is_ok(), "Show in Finder still works");
        let err = attachment_command(&engine, &msg.id, 0, false).unwrap_err();
        assert_eq!(err.code, "opensPrograms");
    }

    #[tokio::test]
    async fn retry_delivers_the_copy_in_the_background() {
        let (engine, rt, _tmp) = setup();
        let bot = create_bot(&engine, input("Researcher")).unwrap();
        let msg = send_message(&engine, &bot.id, None, "hi", &[]).await.unwrap().message;
        wait_for_prompts(&rt, 1).await;
        engine.on_runtime_event(sonata_core::runtime::RuntimeEvent::Exited { pane_id: "w1:p1".into() }).await;
        let copy = retry_message(&engine, &msg.id).await.unwrap();
        assert_eq!(copy.body, "hi");
        wait_for_prompts(&rt, 2).await;
    }

    #[tokio::test]
    async fn retry_delivers_only_in_the_copys_own_session() {
        let (engine, rt, _tmp) = setup();
        let bot = create_bot(&engine, input("Researcher")).unwrap();
        let msg = send_message(&engine, &bot.id, None, "hi", &[]).await.unwrap().message;
        wait_for_prompts(&rt, 1).await;
        engine.on_runtime_event(sonata_core::runtime::RuntimeEvent::Exited { pane_id: "w1:p1".into() }).await;
        engine.start_session_with_message(&bot.id, "other", &[]).unwrap();
        retry_message(&engine, &msg.id).await.unwrap();
        wait_for_prompts(&rt, 2).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        let state = rt.state();
        assert_eq!(state.starts.len(), 2, "only the copy's session started again");
        assert!(state.prompts.iter().all(|(_, text)| !text.contains("other")), "{:?}", state.prompts);
    }

    #[tokio::test]
    async fn pause_round_trips() {
        let (engine, _rt, _tmp) = setup();
        assert!(!get_paused(&engine).unwrap());
        set_paused(&engine, true).await.unwrap();
        assert!(get_paused(&engine).unwrap());
    }

    #[tokio::test]
    async fn deleting_a_bot_moves_its_folder_to_the_trash() {
        let (engine, _rt, _tmp) = setup();
        let bot = create_bot(&engine, input("Researcher")).unwrap();
        let mut trashed = None;
        delete_bot(&engine, &bot.id, |path| {
            trashed = Some(path.to_path_buf());
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(trashed, Some(engine.paths().bots_root.join("researcher")));
        assert_eq!(delete_bot(&engine, &bot.id, |_| Ok(())).await.unwrap_err().code, "botNotFound");
    }

    #[tokio::test]
    async fn removing_a_session_moves_its_drafts_to_the_trash() {
        let (engine, rt, _tmp) = setup();
        let bot = create_bot(&engine, input("Researcher")).unwrap();
        let id = send_message(&engine, &bot.id, None, "hi", &[]).await.unwrap().session.session.id;
        wait_for_prompts(&rt, 1).await;
        let drafts = engine.paths().bots_root.join("researcher/skill-drafts/session-1");
        std::fs::create_dir_all(drafts.join("sonata-seo")).unwrap();
        let mut trashed = Vec::new();
        let removed = remove_session(&engine, &id, |path| {
            trashed.push(path.to_path_buf());
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(removed, vec![id]);
        assert_eq!(trashed, vec![drafts]);
    }

    #[tokio::test]
    async fn drafts_that_cannot_be_trashed_do_not_fail_the_removal() {
        let (engine, rt, _tmp) = setup();
        let bot = create_bot(&engine, input("Researcher")).unwrap();
        let id = send_message(&engine, &bot.id, None, "hi", &[]).await.unwrap().session.session.id;
        wait_for_prompts(&rt, 1).await;
        std::fs::create_dir_all(engine.paths().bots_root.join("researcher/skill-drafts/session-1")).unwrap();
        let mut asked = 0;
        let removed = remove_session(&engine, &id, |_| {
            asked += 1;
            Err("no Trash".into())
        })
        .await
        .unwrap();
        assert_eq!((removed, asked), (vec![id], 1));
        assert!(list_sessions(&engine).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_bot_whose_folder_is_a_dangling_link_still_has_the_link_trashed() {
        let (engine, _rt, tmp) = setup();
        let bot = create_bot(&engine, input("Researcher")).unwrap();
        let folder = engine.paths().bots_root.join("researcher");
        std::fs::remove_dir_all(&folder).unwrap();
        std::os::unix::fs::symlink(tmp.path().join("gone"), &folder).unwrap();
        let mut trashed = None;
        delete_bot(&engine, &bot.id, |path| {
            trashed = Some(path.to_path_buf());
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(trashed, Some(folder), "the link itself goes to the Trash");
    }

    #[tokio::test]
    async fn a_folder_that_cannot_be_trashed_is_reported_after_the_delete() {
        let (engine, _rt, _tmp) = setup();
        let bot = create_bot(&engine, input("Researcher")).unwrap();
        let err = delete_bot(&engine, &bot.id, |_| Err("no Trash".into())).await.unwrap_err();
        assert_eq!((err.code.as_str(), err.params["error"].as_str()), ("trashFailed", Some("no Trash")));
        assert!(list_bots(&engine).await.unwrap().is_empty(), "the bot is deleted all the same");
    }

    #[tokio::test]
    async fn settings_round_trip_and_name_the_folders() {
        let (engine, _rt, tmp) = setup();
        let s = get_settings(&engine).unwrap();
        assert_eq!((s.hop_limit, s.bots_exist, s.setup_done), (12, false, false));
        assert_eq!(s.bots_root, tmp.path().join("bots").to_string_lossy());
        assert_eq!(
            (s.language.as_str(), s.language_resolved.as_str(), s.language_system.as_str()),
            ("system", "en", "en")
        );
        engine.set_system_language("es");
        assert_eq!(set_language(&engine, "system").unwrap(), "es");
        assert_eq!(set_language(&engine, "fr").unwrap(), "fr");
        assert_eq!(set_language(&engine, "pt").unwrap_err().code, "languageUnknown");
        assert_eq!(get_settings(&engine).unwrap().language_system, "es");
        set_hop_limit(&engine, 30).unwrap();
        assert_eq!(set_hop_limit(&engine, 0).unwrap_err().code, "hopLimitRange");
        let chosen = tmp.path().join("chosen");
        set_bots_root(&engine, &chosen.to_string_lossy()).unwrap();
        finish_setup(&engine, None).unwrap();
        let s = get_settings(&engine).unwrap();
        assert_eq!((s.hop_limit, s.setup_done, s.bots_exist), (30, true, true));
        assert_eq!(s.bots_root, chosen.to_string_lossy());
    }

    #[tokio::test]
    async fn finish_setup_takes_an_optional_checkout_and_refuses_one_that_is_not_a_folder() {
        let (engine, _rt, tmp) = setup();
        assert_eq!(finish_setup(&engine, Some("relative")).unwrap_err().code, "badFolder");
        let missing = tmp.path().join("missing");
        assert_eq!(finish_setup(&engine, Some(&missing.to_string_lossy())).unwrap_err().code, "badFolder");
        assert!(list_bots(&engine).await.unwrap().is_empty(), "nothing was made");
        finish_setup(&engine, Some("  ")).unwrap();
        assert_eq!(list_skills(&engine).unwrap().len(), 1);
        assert_eq!(list_bots(&engine).await.unwrap().len(), 1);
        assert!(get_settings(&engine).unwrap().setup_done);
    }

    #[tokio::test]
    async fn settings_show_the_project_folder_and_change_it() {
        let (engine, _rt, tmp) = setup();
        let first = tmp.path().join("sonata");
        std::fs::create_dir(&first).unwrap();
        finish_setup(&engine, Some(&first.to_string_lossy())).unwrap();
        assert_eq!(get_settings(&engine).unwrap().project_folder, Some(first.to_string_lossy().into_owned()));
        let plan = list_bots(&engine).await.unwrap().into_iter().find(|v| v.bot.handle == "master").unwrap().bot;
        update_bot(&engine, &plan.id, CreateBotInput { instructions: "Plan.".into(), ..input("Master") }).unwrap();

        assert_eq!(set_project_folder(&engine, "relative").unwrap_err().code, "badFolder");
        let second = tmp.path().join("sonata-2");
        std::fs::create_dir(&second).unwrap();
        assert_eq!(set_project_folder(&engine, &second.to_string_lossy()).unwrap(), ["master"]);
        assert_eq!(get_settings(&engine).unwrap().project_folder, Some(second.to_string_lossy().into_owned()));
    }

    #[tokio::test]
    async fn the_herdr_status_carries_the_runtimes_answer() {
        let (engine, rt, _tmp) = setup();
        let ok = herdr_status(&engine, Path::new("/app/herdr")).await;
        assert_eq!((ok.version.as_deref(), ok.error), (Some("0.9.3"), None));
        rt.state().health = Err(sonata_core::runtime::RuntimeError::Other("herdr 0.8.0 speaks protocol 19".into()));
        let failed = herdr_status(&engine, Path::new("/app/herdr")).await;
        assert_eq!((failed.path.as_str(), failed.version), ("/app/herdr", None));
        assert!(failed.error.unwrap().contains("protocol 19"));
    }

    #[tokio::test]
    async fn providers_are_listed_claude_first_even_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let found = detect_providers(&tmp.path().to_string_lossy()).await;
        let summary: Vec<_> = found.iter().map(|s| (s.provider, s.path.is_none())).collect();
        assert_eq!(
            summary,
            vec![(Provider::Claude, true), (Provider::Codex, true), (Provider::Grok, true), (Provider::Opencode, true)]
        );
    }

    #[tokio::test]
    async fn an_edit_goes_through_the_engine() {
        let (engine, _rt, _tmp) = setup();
        let bot = create_bot(&engine, input("Researcher")).unwrap();
        let edited =
            update_bot(&engine, &bot.id, CreateBotInput { model: Some("  ".into()), ..input("Scout") }).unwrap();
        assert_eq!((edited.display_name.as_str(), edited.model), ("Scout", None));
    }

    #[tokio::test]
    async fn skills_are_created_listed_attached_and_deleted_to_the_trash() {
        let (engine, _rt, _tmp) = setup();
        let new = NewSkill { name: "Audit".into(), description: "Audits code.".into(), body: "Check.".into() };
        let skill = create_skill(&engine, new).unwrap();
        assert_eq!(get_skill_body(&engine, &skill.id).unwrap(), "Check.");
        let bot =
            create_bot(&engine, CreateBotInput { skill_ids: vec![skill.id.clone()], ..input("Researcher") }).unwrap();
        assert_eq!(bot.skill_ids, vec![skill.id.clone()]);
        assert_eq!(list_skills(&engine).unwrap()[0].used_by, vec!["researcher".to_string()]);
        assert_eq!(delete_skill(&engine, &skill.id, |_| Ok(())).unwrap_err().code, "skillInUse");
        update_bot(&engine, &bot.id, input("Researcher")).unwrap();
        let mut trashed = None;
        delete_skill(&engine, &skill.id, |path| {
            trashed = Some(path.to_path_buf());
            Ok(())
        })
        .unwrap();
        assert_eq!(trashed, Some(engine.skills_dir().join("audit")));
        let blank = NewSkill { name: " ".into(), description: "x".into(), body: String::new() };
        assert_eq!(create_skill(&engine, blank).unwrap_err().code, "skillNameRequired");
        assert_eq!(import_skill(&engine, "/nowhere").unwrap_err().code, "skillNoFile");
    }

    #[tokio::test]
    async fn a_skill_folder_that_cannot_be_trashed_is_reported_as_a_skill_after_the_delete() {
        let (engine, _rt, _tmp) = setup();
        let new = NewSkill { name: "Audit".into(), description: "Audits code.".into(), body: "Check.".into() };
        let skill = create_skill(&engine, new).unwrap();
        let err = delete_skill(&engine, &skill.id, |_| Err("no Trash".into())).unwrap_err();
        assert_eq!((err.code.as_str(), err.params["error"].as_str()), ("skillTrashFailed", Some("no Trash")));
        assert_eq!(err.params["path"].as_str(), Some(engine.skills_dir().join("audit").to_string_lossy().as_ref()));
        assert!(list_skills(&engine).unwrap().is_empty(), "the skill is deleted all the same");
    }

    #[test]
    fn folders_open_in_finder() {
        let command = finder(Path::new("/x/bots"));
        assert_eq!(command.get_program(), if cfg!(target_os = "macos") { "/usr/bin/open" } else { "xdg-open" });
        assert_eq!(command.get_args().collect::<Vec<_>>(), vec!["/x/bots"]);
    }
}
