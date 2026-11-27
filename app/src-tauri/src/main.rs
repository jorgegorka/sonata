#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod browser;
mod browser_pick;
mod commands;
mod language;
mod locale;
mod logging;
mod menu;
mod notify;
mod omarchy_theme;
mod platform;
mod startup;
#[cfg(test)]
mod task_commands_tests;
mod tray;
mod updates;

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use commands::{AppSettings, CommandError, CreateBotInput, HerdrStatus};
use sonata_core::detect::{self, ProviderStatus};
use sonata_core::engine::{Engine, EngineConfig, ExportSummary, ImportMode, ImportOutcome, ImportPreview, UiEvent};
use sonata_core::engine::{OnboardingApplyResult, OnboardingPreview, OnboardingProgress, OnboardingState};
use sonata_core::engine::{WorkflowFileEntry, WorkflowImportOutcome, WorkflowImportPreview, WorkflowImportRequest};
use sonata_core::model::{
    Bot, BotView, Glance, Message, NewSkill, NewTask, NewWorkflow, Project, ProjectInput, Provider, RunView,
    SessionActivity, SessionView, Skill, SkillView, Task, ThreadSummary, UsageLimitView, Workflow,
};
use sonata_core::paths::{resolve_bots_root, AppPaths};
use sonata_core::runtime::herdr::{EnvSource, HerdrRuntime};
use sonata_core::store::Store;
use startup::StartupError;
use tauri::{Emitter, Manager, RunEvent, State};
use tokio::signal::unix::SignalKind;

const LOGIN_SHELL_LIMIT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub struct UserEnv {
    pub search_path: String,
    pub server_env: Vec<(String, String)>,
}

async fn user_env(shell: PathBuf) -> UserEnv {
    let login = detect::login_shell_env(&shell, LOGIN_SHELL_LIMIT).await;
    match &login {
        Some(env) => tracing::info!(shell = %shell.display(), path = %env.path, "read the login shell's PATH"),
        None => {
            tracing::warn!(shell = %shell.display(), "could not read the login shell's PATH; using the usual folders")
        }
    }
    let home = dirs::home_dir().unwrap_or_default();
    let search_path =
        detect::search_path(login.as_ref().map(|e| e.path.as_str()), &home, std::env::var("PATH").ok().as_deref());
    let apple_locale = detect::apple_locale().await;
    let lang = detect::utf8_lang(login.as_ref().and_then(|e| e.lang.as_deref()), apple_locale.as_deref());
    tracing::info!(lang = %lang, "bots run with this LANG");
    let server_env = vec![
        ("PATH".to_string(), search_path.clone()),
        ("SHELL".to_string(), shell.display().to_string()),
        ("LANG".to_string(), lang),
    ];
    UserEnv { search_path, server_env }
}

pub struct AppEnv {
    shell: PathBuf,
    user: Arc<tokio::sync::OnceCell<UserEnv>>,
    pub herdr: PathBuf,
}

impl AppEnv {
    pub async fn user(&self) -> UserEnv {
        let shell = self.shell.clone();
        self.user.get_or_init(|| user_env(shell)).await.clone()
    }
}

const WATCHDOG_EVERY: Duration = Duration::from_secs(15);
const SCHEDULER_EVERY: Duration = Duration::from_secs(60);

fn window_in_front(app: &tauri::AppHandle) -> bool {
    app.get_window(browser::MAIN_WINDOW)
        .map(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false))
        .unwrap_or(false)
}

#[tauri::command]
fn list_projects(engine: State<'_, Engine>) -> Result<Vec<Project>, CommandError> {
    Ok(engine.list_projects()?)
}
#[tauri::command]
fn create_project(engine: State<'_, Engine>, input: ProjectInput) -> Result<Project, CommandError> {
    Ok(engine.create_project(input)?)
}
#[tauri::command]
fn update_project(engine: State<'_, Engine>, project_id: String, input: ProjectInput) -> Result<Project, CommandError> {
    Ok(engine.update_project(&project_id, input)?)
}
#[tauri::command]
async fn archive_project(engine: State<'_, Engine>, project_id: String) -> Result<Project, CommandError> {
    Ok(engine.archive_project(&project_id).await?)
}
#[tauri::command]
fn restore_project(engine: State<'_, Engine>, project_id: String) -> Result<Project, CommandError> {
    Ok(engine.restore_project(&project_id)?)
}
#[tauri::command]
fn finish_project_setup(engine: State<'_, Engine>, input: ProjectInput) -> Result<Project, CommandError> {
    Ok(engine.finish_project_setup(input)?)
}

#[tauri::command]
async fn list_work_items(
    engine: State<'_, Engine>,
    project_id: Option<String>,
) -> Result<Vec<sonata_core::work_items::WorkItem>, CommandError> {
    Ok(engine.request_project(project_id.as_deref(), &[])?.list_work_items().await?)
}
#[tauri::command]
async fn create_work_item(
    engine: State<'_, Engine>,
    project_id: Option<String>,
    input: sonata_core::work_items::NewWorkItem,
) -> Result<sonata_core::work_items::WorkItem, CommandError> {
    Ok(engine.request_project(project_id.as_deref(), &[])?.create_work_item(input)?)
}
#[tauri::command]
async fn start_work_item(
    engine: State<'_, Engine>,
    project_id: Option<String>,
    task_id: String,
) -> Result<(), CommandError> {
    Ok(engine.request_project(project_id.as_deref(), &[("work_items", &task_id)])?.start_work_item(&task_id)?)
}
#[tauri::command]
fn archive_work_item(
    engine: State<'_, Engine>,
    project_id: Option<String>,
    task_id: String,
    archived: bool,
) -> Result<(), CommandError> {
    Ok(engine
        .request_project(project_id.as_deref(), &[("work_items", &task_id)])?
        .archive_work_item(&task_id, archived)?)
}
#[tauri::command]
async fn cancel_work_item(
    engine: State<'_, Engine>,
    project_id: Option<String>,
    task_id: String,
) -> Result<(), CommandError> {
    Ok(engine.request_project(project_id.as_deref(), &[("work_items", &task_id)])?.cancel_work_item(&task_id).await?)
}

#[tauri::command]
fn list_orchestrations(
    engine: State<'_, Engine>,
    project_id: Option<String>,
) -> Result<Vec<sonata_core::orchestration::OrchestrationView>, CommandError> {
    Ok(engine.request_project(project_id.as_deref(), &[])?.list_orchestrations()?)
}

#[tauri::command]
async fn cancel_orchestration(
    engine: State<'_, Engine>,
    run_id: String,
    task: Option<String>,
    project_id: Option<String>,
) -> Result<(), CommandError> {
    Ok(engine
        .request_project(project_id.as_deref(), &[("orchestrations", &run_id)])?
        .cancel_orchestration(&run_id, task.as_deref())
        .await?)
}

#[tauri::command]
async fn archive_orchestration_task(
    engine: State<'_, Engine>,
    run_id: String,
    task: String,
    archived: bool,
    project_id: Option<String>,
) -> Result<(), CommandError> {
    Ok(engine
        .request_project(project_id.as_deref(), &[("orchestrations", &run_id)])?
        .archive_orchestration_task(&run_id, &task, archived)
        .await?)
}

#[tauri::command]
fn get_onboarding(engine: State<'_, Engine>, project_id: Option<String>) -> Result<OnboardingState, CommandError> {
    Ok(engine.request_project(project_id.as_deref(), &[])?.onboarding_state()?)
}
#[tauri::command]
fn save_onboarding(
    engine: State<'_, Engine>,
    project_id: Option<String>,
    progress: OnboardingProgress,
) -> Result<OnboardingState, CommandError> {
    Ok(engine.request_project(project_id.as_deref(), &[])?.save_onboarding(progress)?)
}
#[tauri::command]
async fn generate_onboarding(
    engine: State<'_, Engine>,
    project_id: Option<String>,
) -> Result<OnboardingState, CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[])?;
    let state = engine.start_onboarding()?;
    if let Some(session) = &state.session_id {
        tokio::spawn(engine.pump_session(session));
    }
    Ok(state)
}
#[tauri::command]
async fn cancel_onboarding(
    engine: State<'_, Engine>,
    project_id: Option<String>,
) -> Result<OnboardingState, CommandError> {
    Ok(engine.request_project(project_id.as_deref(), &[])?.cancel_onboarding().await?)
}
#[tauri::command]
fn preview_onboarding(
    engine: State<'_, Engine>,
    project_id: Option<String>,
    selected: Vec<String>,
) -> Result<OnboardingPreview, CommandError> {
    Ok(engine.request_project(project_id.as_deref(), &[])?.preview_onboarding(selected)?)
}
#[tauri::command]
fn apply_onboarding(
    engine: State<'_, Engine>,
    project_id: Option<String>,
    selected: Vec<String>,
    token: String,
) -> Result<OnboardingApplyResult, CommandError> {
    Ok(engine.request_project(project_id.as_deref(), &[])?.apply_onboarding(selected, &token)?)
}
#[tauri::command]
async fn finish_onboarding(engine: State<'_, Engine>, project_id: Option<String>) -> Result<(), CommandError> {
    Ok(engine.request_project(project_id.as_deref(), &[])?.finish_onboarding().await?)
}

#[tauri::command]
async fn list_bots(engine: State<'_, Engine>, project_id: Option<String>) -> Result<Vec<BotView>, CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[])?;
    commands::list_bots(&engine).await
}

#[tauri::command]
async fn create_bot(
    engine: State<'_, Engine>,
    input: CreateBotInput,
    project_id: Option<String>,
) -> Result<Bot, CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[])?;
    commands::create_bot(&engine, input)
}

#[tauri::command]
async fn list_sessions(
    engine: State<'_, Engine>,
    project_id: Option<String>,
) -> Result<Vec<SessionView>, CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[])?;
    commands::list_sessions(&engine).await
}

#[tauri::command]
async fn get_session_messages(
    engine: State<'_, Engine>,
    session_id: String,
    project_id: Option<String>,
) -> Result<Vec<Message>, CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[("sessions", session_id.as_str())])?;
    commands::get_session_messages(&engine, &session_id)
}

#[tauri::command]
async fn send_message(
    engine: State<'_, Engine>,
    bot_id: String,
    session_id: Option<String>,
    body: String,
    files: Vec<String>,
    project_id: Option<String>,
) -> Result<commands::Sent, CommandError> {
    let engine = engine.request_project(
        project_id.as_deref(),
        &[("bots", bot_id.as_str()), ("sessions", session_id.as_deref().unwrap_or(""))],
    )?;
    commands::send_message(&engine, &bot_id, session_id.as_deref(), &body, &files).await
}

#[tauri::command]
async fn request_workflow(
    engine: State<'_, Engine>,
    body: String,
    project_id: Option<String>,
) -> Result<commands::Sent, CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[])?;
    commands::request_workflow(&engine, &body).await
}

#[tauri::command]
async fn cancel_message(
    engine: State<'_, Engine>,
    message_id: String,
    project_id: Option<String>,
) -> Result<Message, CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[("messages", message_id.as_str())])?;
    commands::cancel_message(&engine, &message_id).await
}

#[tauri::command]
async fn retry_message(
    engine: State<'_, Engine>,
    message_id: String,
    project_id: Option<String>,
) -> Result<Message, CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[("messages", message_id.as_str())])?;
    commands::retry_message(&engine, &message_id).await
}

#[tauri::command]
async fn stop_turn(
    engine: State<'_, Engine>,
    session_id: String,
    project_id: Option<String>,
) -> Result<(), CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[("sessions", session_id.as_str())])?;
    commands::stop_turn(&engine, &session_id).await
}

#[tauri::command]
async fn answer_dialog(
    engine: State<'_, Engine>,
    session_id: String,
    answer: String,
    project_id: Option<String>,
) -> Result<(), CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[("sessions", session_id.as_str())])?;
    commands::answer_dialog(&engine, &session_id, &answer).await
}

#[tauri::command]
async fn session_snapshot(
    engine: State<'_, Engine>,
    session_id: String,
    project_id: Option<String>,
) -> Result<Option<String>, CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[("sessions", session_id.as_str())])?;
    commands::session_snapshot(&engine, &session_id).await
}

#[tauri::command]
async fn session_terminal(
    engine: State<'_, Engine>,
    session_id: String,
    project_id: Option<String>,
) -> Result<Option<String>, CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[("sessions", session_id.as_str())])?;
    commands::session_terminal(&engine, &session_id).await
}

#[tauri::command]
async fn restart_session(
    engine: State<'_, Engine>,
    session_id: String,
    project_id: Option<String>,
) -> Result<(), CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[("sessions", session_id.as_str())])?;
    commands::restart_session(&engine, &session_id).await
}

#[tauri::command]
async fn stop_session(
    engine: State<'_, Engine>,
    session_id: String,
    project_id: Option<String>,
) -> Result<(), CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[("sessions", session_id.as_str())])?;
    commands::stop_session(&engine, &session_id).await
}

#[tauri::command]
async fn remove_session(
    engine: State<'_, Engine>,
    session_id: String,
    project_id: Option<String>,
) -> Result<Vec<String>, CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[("sessions", session_id.as_str())])?;
    commands::remove_session(&engine, &session_id, commands::move_to_trash).await
}

#[tauri::command]
async fn rename_session(
    engine: State<'_, Engine>,
    session_id: String,
    name: String,
    project_id: Option<String>,
) -> Result<SessionView, CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[("sessions", session_id.as_str())])?;
    commands::rename_session(&engine, &session_id, &name).await
}

#[tauri::command]
async fn mark_session_seen(
    engine: State<'_, Engine>,
    session_id: String,
    project_id: Option<String>,
) -> Result<(), CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[("sessions", session_id.as_str())])?;
    commands::mark_session_seen(&engine, &session_id)
}

#[tauri::command]
async fn session_activity(
    engine: State<'_, Engine>,
    session_ids: Vec<String>,
    project_id: Option<String>,
) -> Result<Vec<SessionActivity>, CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[])?;
    commands::session_activity(&engine, &session_ids).await
}

#[tauri::command]
async fn session_glances(
    engine: State<'_, Engine>,
    session_ids: Vec<String>,
    project_id: Option<String>,
) -> Result<Vec<Glance>, CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[])?;
    Ok(commands::session_glances(&engine, &session_ids).await)
}

#[tauri::command]
async fn archive_session(
    engine: State<'_, Engine>,
    session_id: String,
    project_id: Option<String>,
) -> Result<Vec<String>, CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[("sessions", session_id.as_str())])?;
    commands::archive_session(&engine, &session_id).await
}

#[tauri::command]
async fn restore_session(
    engine: State<'_, Engine>,
    session_id: String,
    project_id: Option<String>,
) -> Result<Vec<String>, CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[("sessions", session_id.as_str())])?;
    commands::restore_session(&engine, &session_id).await
}

#[tauri::command]
async fn list_tasks(engine: State<'_, Engine>, project_id: Option<String>) -> Result<Vec<Task>, CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[])?;
    commands::list_tasks(&engine)
}

#[tauri::command]
async fn create_task(
    engine: State<'_, Engine>,
    input: NewTask,
    project_id: Option<String>,
) -> Result<Task, CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[])?;
    commands::create_task(&engine, input)
}

#[tauri::command]
async fn update_task(
    engine: State<'_, Engine>,
    task_id: String,
    input: NewTask,
    project_id: Option<String>,
) -> Result<Task, CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[("tasks", task_id.as_str())])?;
    commands::update_task(&engine, &task_id, input)
}

#[tauri::command]
async fn set_task_enabled(
    engine: State<'_, Engine>,
    task_id: String,
    enabled: bool,
    project_id: Option<String>,
) -> Result<Task, CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[("tasks", task_id.as_str())])?;
    commands::set_task_enabled(&engine, &task_id, enabled)
}

#[tauri::command]
async fn delete_task(
    engine: State<'_, Engine>,
    task_id: String,
    project_id: Option<String>,
) -> Result<(), CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[("tasks", task_id.as_str())])?;
    commands::delete_task(&engine, &task_id)
}

#[tauri::command]
async fn run_task_now(
    engine: State<'_, Engine>,
    task_id: String,
    project_id: Option<String>,
) -> Result<SessionView, CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[("tasks", task_id.as_str())])?;
    commands::run_task_now(&engine, &task_id).await
}

#[tauri::command]
async fn list_workflows(engine: State<'_, Engine>, project_id: Option<String>) -> Result<Vec<Workflow>, CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[])?;
    commands::list_workflows(&engine)
}

#[tauri::command]
async fn create_workflow(
    engine: State<'_, Engine>,
    input: NewWorkflow,
    project_id: Option<String>,
) -> Result<Workflow, CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[])?;
    commands::create_workflow(&engine, input)
}

#[tauri::command]
async fn update_workflow(
    engine: State<'_, Engine>,
    workflow_id: String,
    input: NewWorkflow,
    project_id: Option<String>,
) -> Result<Workflow, CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[("workflows", workflow_id.as_str())])?;
    commands::update_workflow(&engine, &workflow_id, input)
}

#[tauri::command]
async fn delete_workflow(
    engine: State<'_, Engine>,
    workflow_id: String,
    project_id: Option<String>,
) -> Result<(), CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[("workflows", workflow_id.as_str())])?;
    commands::delete_workflow(&engine, &workflow_id)
}

#[tauri::command]
async fn start_workflow(
    engine: State<'_, Engine>,
    workflow_id: String,
    body: String,
    files: Vec<String>,
    project_id: Option<String>,
) -> Result<SessionView, CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[("workflows", workflow_id.as_str())])?;
    commands::start_workflow(&engine, &workflow_id, &body, &files).await
}

#[tauri::command]
async fn get_workflow_run(
    engine: State<'_, Engine>,
    run_id: String,
    project_id: Option<String>,
) -> Result<RunView, CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[("workflow_runs", run_id.as_str())])?;
    commands::get_workflow_run(&engine, &run_id)
}

#[tauri::command]
async fn another_round(
    engine: State<'_, Engine>,
    run_id: String,
    project_id: Option<String>,
) -> Result<(), CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[("workflow_runs", run_id.as_str())])?;
    commands::another_round(&engine, &run_id)
}

#[tauri::command]
async fn pick_outcome(
    engine: State<'_, Engine>,
    run_id: String,
    outcome: Option<String>,
    project_id: Option<String>,
) -> Result<(), CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[("workflow_runs", run_id.as_str())])?;
    commands::pick_outcome(&engine, &run_id, outcome)
}

#[tauri::command]
async fn remind_step(
    engine: State<'_, Engine>,
    run_id: String,
    project_id: Option<String>,
) -> Result<(), CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[("workflow_runs", run_id.as_str())])?;
    commands::remind_step(&engine, &run_id)
}

#[tauri::command]
async fn stop_run(engine: State<'_, Engine>, run_id: String, project_id: Option<String>) -> Result<(), CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[("workflow_runs", run_id.as_str())])?;
    commands::stop_run(&engine, &run_id).await
}

#[tauri::command]
async fn get_paused(engine: State<'_, Engine>) -> Result<bool, CommandError> {
    commands::get_paused(&engine)
}

#[tauri::command]
async fn set_paused(engine: State<'_, Engine>, paused: bool) -> Result<(), CommandError> {
    commands::set_paused(&engine, paused).await
}

#[tauri::command]
async fn usage_limits(engine: State<'_, Engine>) -> Result<Vec<UsageLimitView>, CommandError> {
    commands::usage_limits(&engine)
}

#[tauri::command]
async fn try_usage_limit_now(engine: State<'_, Engine>, provider: Provider) -> Result<(), CommandError> {
    commands::try_usage_limit_now(&engine, provider).await
}

#[tauri::command]
async fn list_bot_threads(
    engine: State<'_, Engine>,
    project_id: Option<String>,
) -> Result<Vec<ThreadSummary>, CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[])?;
    commands::list_bot_threads(&engine)
}

#[tauri::command]
async fn get_thread_messages(
    engine: State<'_, Engine>,
    thread_id: String,
    project_id: Option<String>,
) -> Result<Vec<Message>, CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[("threads", thread_id.as_str())])?;
    commands::get_thread_messages(&engine, &thread_id)
}

#[tauri::command]
async fn open_attachment(
    engine: State<'_, Engine>,
    message_id: String,
    index: usize,
    reveal: bool,
    project_id: Option<String>,
) -> Result<(), CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[("messages", message_id.as_str())])?;
    commands::open_attachment(&engine, &message_id, index, reveal).await
}

#[tauri::command]
async fn update_bot(
    engine: State<'_, Engine>,
    bot_id: String,
    input: CreateBotInput,
    project_id: Option<String>,
) -> Result<Bot, CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[("bots", bot_id.as_str())])?;
    commands::update_bot(&engine, &bot_id, input)
}

#[tauri::command]
async fn delete_bot(engine: State<'_, Engine>, bot_id: String, project_id: Option<String>) -> Result<(), CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[("bots", bot_id.as_str())])?;
    commands::delete_bot(&engine, &bot_id, commands::move_to_trash).await
}

#[tauri::command]
async fn open_bot_folder(
    engine: State<'_, Engine>,
    bot_id: String,
    project_id: Option<String>,
) -> Result<(), CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[("bots", bot_id.as_str())])?;
    commands::open_bot_folder(&engine, &bot_id).await
}

#[tauri::command]
async fn open_bots_folder(engine: State<'_, Engine>) -> Result<(), CommandError> {
    commands::open_bots_folder(&engine).await
}

#[tauri::command]
async fn open_logs_folder(engine: State<'_, Engine>) -> Result<(), CommandError> {
    commands::open_logs_folder(&engine).await
}

#[tauri::command]
async fn open_project_folder(engine: State<'_, Engine>, project_id: Option<String>) -> Result<(), CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[])?;
    commands::open_project_folder(&engine).await
}

#[tauri::command]
async fn list_skills(engine: State<'_, Engine>) -> Result<Vec<SkillView>, CommandError> {
    commands::list_skills(&engine)
}

#[tauri::command]
async fn create_skill(engine: State<'_, Engine>, input: NewSkill) -> Result<Skill, CommandError> {
    commands::create_skill(&engine, input)
}

#[tauri::command]
async fn import_skill(engine: State<'_, Engine>, path: String) -> Result<Skill, CommandError> {
    commands::import_skill(&engine, &path)
}

async fn blocking<T: Send + 'static>(
    job: impl FnOnce() -> Result<T, CommandError> + Send + 'static,
) -> Result<T, CommandError> {
    tauri::async_runtime::spawn_blocking(job).await.map_err(CommandError::internal)?
}

#[tauri::command]
async fn export_config(
    engine: State<'_, Engine>,
    path: String,
    project_id: Option<String>,
) -> Result<ExportSummary, CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[])?;

    blocking(move || commands::export_config(&engine, &path)).await
}

#[tauri::command]
async fn export_workflow_config(
    engine: State<'_, Engine>,
    workflow_id: String,
    path: String,
    project_id: Option<String>,
) -> Result<ExportSummary, CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[("workflows", workflow_id.as_str())])?;

    blocking(move || commands::export_workflow_config(&engine, &workflow_id, &path)).await
}

#[tauri::command]
async fn workflows_in_config(engine: State<'_, Engine>, path: String) -> Result<Vec<WorkflowFileEntry>, CommandError> {
    let engine = engine.inner().clone();
    blocking(move || commands::workflows_in_config(&engine, &path)).await
}

#[tauri::command]
async fn preview_workflow_import(
    engine: State<'_, Engine>,
    path: String,
    request: WorkflowImportRequest,
    project_id: Option<String>,
) -> Result<WorkflowImportPreview, CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[])?;

    blocking(move || commands::preview_workflow_import(&engine, &path, &request)).await
}

#[tauri::command]
async fn import_workflow_config(
    engine: State<'_, Engine>,
    path: String,
    request: WorkflowImportRequest,
    token: String,
    project_id: Option<String>,
) -> Result<WorkflowImportOutcome, CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[])?;

    blocking(move || commands::import_workflow_config(&engine, &path, &request, &token)).await
}

#[tauri::command]
async fn preview_config_import(
    engine: State<'_, Engine>,
    path: String,
    project_id: Option<String>,
) -> Result<ImportPreview, CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[])?;

    blocking(move || commands::preview_config_import(&engine, &path)).await
}

#[tauri::command]
async fn import_config(
    engine: State<'_, Engine>,
    path: String,
    mode: ImportMode,
    token: String,
    project_id: Option<String>,
) -> Result<ImportOutcome, CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[])?;

    blocking(move || commands::import_config(&engine, &path, mode, &token)).await
}

#[tauri::command]
async fn get_skill_body(engine: State<'_, Engine>, skill_id: String) -> Result<String, CommandError> {
    commands::get_skill_body(&engine, &skill_id)
}

#[tauri::command]
async fn update_skill(engine: State<'_, Engine>, skill_id: String, input: NewSkill) -> Result<Skill, CommandError> {
    commands::update_skill(&engine, &skill_id, input)
}

#[tauri::command]
async fn delete_skill(engine: State<'_, Engine>, skill_id: String) -> Result<(), CommandError> {
    commands::delete_skill(&engine, &skill_id, commands::move_to_trash)
}

#[tauri::command]
async fn open_skills_folder(engine: State<'_, Engine>) -> Result<(), CommandError> {
    commands::open_skills_folder(&engine).await
}

#[tauri::command]
async fn get_settings(engine: State<'_, Engine>, project_id: Option<String>) -> Result<AppSettings, CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[])?;
    commands::get_settings(&engine)
}

#[tauri::command]
async fn set_hop_limit(engine: State<'_, Engine>, limit: i64) -> Result<(), CommandError> {
    commands::set_hop_limit(&engine, limit)
}

#[tauri::command]
async fn set_language(
    app: tauri::AppHandle,
    engine: State<'_, Engine>,
    choice: String,
) -> Result<String, CommandError> {
    let resolved = commands::set_language(&engine, &choice)?;
    locale::set_language(&resolved);
    let menu = menu::build(&app).map_err(CommandError::internal)?;
    app.set_menu(menu).map_err(CommandError::internal)?;
    tray::rebuild(&app).map_err(CommandError::internal)?;
    Ok(resolved)
}

#[tauri::command]
async fn set_browser_shown(app: tauri::AppHandle, shown: bool) -> Result<(), CommandError> {
    menu::set_browser_shown(&app, shown).map_err(CommandError::internal)
}

#[tauri::command]
async fn set_bots_root(engine: State<'_, Engine>, path: String) -> Result<(), CommandError> {
    commands::set_bots_root(&engine, &path)
}

#[tauri::command]
async fn set_project_folder(
    engine: State<'_, Engine>,
    path: String,
    project_id: Option<String>,
) -> Result<Vec<String>, CommandError> {
    let engine = engine.request_project(project_id.as_deref(), &[])?;
    commands::set_project_folder(&engine, &path)
}

#[tauri::command]
async fn finish_setup(engine: State<'_, Engine>, project_folder: Option<String>) -> Result<(), CommandError> {
    commands::finish_setup(&engine, project_folder.as_deref())
}

#[tauri::command]
async fn detect_providers(env: State<'_, AppEnv>) -> Result<Vec<ProviderStatus>, CommandError> {
    Ok(commands::detect_providers(&env.user().await.search_path).await)
}

#[tauri::command]
async fn herdr_status(engine: State<'_, Engine>, env: State<'_, AppEnv>) -> Result<HerdrStatus, CommandError> {
    Ok(commands::herdr_status(&engine, &env.herdr).await)
}

fn hint_once(app: &tauri::AppHandle) {
    let engine = app.state::<Engine>();
    match engine.close_hint_shown() {
        Ok(true) => {}
        Ok(false) => {
            if let Err(e) = engine.mark_close_hint_shown() {
                tracing::warn!(error = %e, "could not record the close hint");
            }
            let notice = notify::Notice {
                title: locale::t("notifications.hiddenTitle", &[]),
                body: locale::t("notifications.hiddenBody", &[]),
            };
            notify::send(app, &notice);
        }
        Err(e) => tracing::warn!(error = %e, "could not read the close hint setting"),
    }
}

fn build_engine(
    paths: &AppPaths,
    preferred: &[String],
    system_language: &'static str,
    resources: Option<&Path>,
) -> Result<(Engine, File, AppEnv), StartupError> {
    let lock = paths.lock().map_err(startup::lock_error)?;
    let store = Store::open(&paths.db_path()).map_err(startup::store_error)?;
    let choice = store.get_setting("language").map_err(startup::store_error)?.unwrap_or_default();
    locale::set_language(language::resolve(&choice, system_language));
    let mut paths = paths.clone();
    let saved = store.get_setting("bots_root").map_err(startup::store_error)?;
    paths.bots_root =
        resolve_bots_root(std::env::var_os("SONATA_BOTS_ROOT").map(PathBuf::from), saved, paths.bots_root);
    let executable = std::env::current_exe().unwrap_or_default();
    let runtime_dir = platform::runtime_dir(&executable, resources);
    let herdr = std::env::var_os("SONATA_HERDR_BIN").map(PathBuf::from).unwrap_or_else(|| runtime_dir.join("herdr"));
    let herdr = Some(herdr).filter(|p| p.is_file()).ok_or_else(|| {
        StartupError::new("herdrMissing", vec![], "no herdr next to the executable (dev runs set SONATA_HERDR_BIN)")
    })?;
    let sbot_dir = Some(runtime_dir).filter(|dir| dir.join("sbot").is_file()).ok_or_else(|| {
        StartupError::new("sbotMissing", vec![], "no sbot next to the executable (dev runs: cargo build -p sbot)")
    })?;
    tracing::info!(herdr = %herdr.display(), sbot = %sbot_dir.display(), "found herdr and sbot");
    let shell = detect::user_shell();
    let user = Arc::new(tokio::sync::OnceCell::new());
    let source: EnvSource = {
        let (cell, shell) = (user.clone(), shell.clone());
        Box::pin(async move { cell.get_or_init(|| user_env(shell)).await.server_env.clone() })
    };
    let engine = Engine::new(
        store,
        HerdrRuntime::new_with_env_source(herdr.clone(), paths.herdr_session(), source),
        EngineConfig {
            paths: paths.clone(),
            sbot_dir,
            fallback_lines: 200,
            turn_start_grace: Some(Duration::from_secs(10)),
        },
    );
    engine.set_system_language(system_language);
    engine.set_trash(Arc::new(commands::move_to_trash));
    let language = engine.language().map_err(startup::store_error)?;
    locale::set_language(&language);
    tracing::info!(language, system = system_language, ?preferred, "UI language");
    if let Err(e) = engine.ensure_default_bots() {
        tracing::warn!(error = %e, "could not complete the default setup");
    }
    Ok((engine, lock, AppEnv { shell, user, herdr }))
}

#[tauri::command]
async fn browser_open(
    app: tauri::AppHandle,
    url: String,
    rect: browser::PanelRect,
) -> Result<browser::PageState, CommandError> {
    browser::open(&app, &url, rect)
}

#[tauri::command]
async fn browser_place(app: tauri::AppHandle, rect: browser::PanelRect) -> Result<(), CommandError> {
    browser::place(&app, rect)
}

#[tauri::command]
async fn browser_close(app: tauri::AppHandle) -> Result<(), CommandError> {
    browser::close(&app)
}

#[tauri::command]
async fn browser_navigate(
    app: tauri::AppHandle,
    address: String,
    rect: browser::PanelRect,
) -> Result<(), CommandError> {
    browser::navigate(&app, &address, rect)
}

#[tauri::command]
async fn browser_show(
    app: tauri::AppHandle,
    rect: browser::PanelRect,
) -> Result<Option<browser::PageState>, CommandError> {
    browser::reveal(&app, rect)
}

#[tauri::command]
async fn browser_back(app: tauri::AppHandle) -> Result<(), CommandError> {
    browser::back(&app)
}

#[tauri::command]
async fn browser_forward(app: tauri::AppHandle) -> Result<(), CommandError> {
    browser::forward(&app)
}

#[tauri::command]
async fn browser_reload(app: tauri::AppHandle) -> Result<(), CommandError> {
    browser::reload(&app)
}

#[tauri::command]
async fn browser_pick_start(
    app: tauri::AppHandle,
    strings: browser_pick::PickStrings,
    target: Option<String>,
) -> Result<(), CommandError> {
    browser_pick::start(&app, strings, target)
}

#[tauri::command]
async fn browser_pick_stop(app: tauri::AppHandle) -> Result<(), CommandError> {
    browser_pick::stop(&app)
}

#[tauri::command]
async fn browser_pick_target(app: tauri::AppHandle, target: Option<String>) -> Result<(), CommandError> {
    browser_pick::set_target(&app, target)
}

#[tauri::command]
async fn browser_pick_result(app: tauri::AppHandle, ok: bool) -> Result<(), CommandError> {
    browser_pick::report(&app, ok)
}

#[tauri::command]
async fn browser_pick_box_state(app: tauri::AppHandle) -> Result<browser_pick::PickBoxState, CommandError> {
    Ok(browser_pick::box_state(&app))
}

#[tauri::command]
async fn browser_pick_submit(app: tauri::AppHandle, comment: String) -> Result<(), CommandError> {
    browser_pick::submit(&app, &comment)
}

#[tauri::command]
async fn browser_pick_close(app: tauri::AppHandle) -> Result<(), CommandError> {
    browser_pick::close_box(&app)
}

#[tauri::command]
fn omarchy_theme() -> Option<omarchy_theme::Palette> {
    omarchy_theme::read_current()
}

fn main() {
    let context = tauri::generate_context!();
    let resources = tauri::utils::platform::resource_dir(context.package_info(), &tauri::utils::Env::default()).ok();
    let preferred = tauri::async_runtime::block_on(language::preferred_languages());
    let system_language = language::system_language(&preferred);
    locale::set_language(system_language);
    let paths = AppPaths::from_env_or_default();
    if let Err(e) = paths.ensure_data() {
        let vars = vec![("path", paths.data_dir.display().to_string()), ("error", e.to_string())];
        startup::exit_with(StartupError::new("folders", vars, e.to_string()));
    }
    if let Err(message) = logging::init(&paths.logs_dir()) {
        startup::exit_with(StartupError::new("logging", vec![("error", message.clone())], message));
    }
    tracing::info!(data_dir = %paths.data_dir.display(), "Sonata is starting");
    let (engine, _lock, app_env) = build_engine(&paths, &preferred, system_language, resources.as_deref())
        .unwrap_or_else(|e| startup::exit_with(e));
    let socket = paths.bridge_socket();
    let bridge = tauri::async_runtime::block_on(async {
        engine.run_event_loop();
        engine.run_watchdog(WATCHDOG_EVERY);
        sonata_core::bridge::serve(&socket, engine.clone())
    });
    if let Err(e) = bridge {
        let vars = vec![("path", socket.display().to_string()), ("error", e.to_string())];
        startup::exit_with(StartupError::new("bridge", vars, e.to_string()));
    }
    let for_setup = engine.clone();
    let for_exit = engine.clone();

    tauri::Builder::default()
        .menu(menu::build)
        .on_menu_event(|app, event| {
            if let Some(ui_event) = menu::ui_event_for(event.id().as_ref()) {
                tray::show_window(app);
                let _ = app.emit(ui_event, ());
            }
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                #[cfg(target_os = "macos")]
                let _ = window.hide();
                #[cfg(not(target_os = "macos"))]
                let _ = window.minimize();
                hint_once(window.app_handle());
            }
        })
        .plugin(tauri_plugin_opener::Builder::new().open_js_links_on_click(false).build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(updates::Updates::default())
        .manage(engine)
        .manage(app_env)
        .manage(browser::Browser::default())
        .manage(menu::ViewState::default())
        .invoke_handler(tauri::generate_handler![
            list_projects,
            list_orchestrations,
            list_work_items,
            create_work_item,
            start_work_item,
            archive_work_item,
            cancel_work_item,
            cancel_orchestration,
            archive_orchestration_task,
            create_project,
            update_project,
            archive_project,
            restore_project,
            finish_project_setup,
            get_onboarding,
            save_onboarding,
            generate_onboarding,
            cancel_onboarding,
            preview_onboarding,
            apply_onboarding,
            finish_onboarding,
            list_bots,
            create_bot,
            list_tasks,
            create_task,
            update_task,
            set_task_enabled,
            delete_task,
            run_task_now,
            list_workflows,
            create_workflow,
            update_workflow,
            delete_workflow,
            start_workflow,
            get_workflow_run,
            another_round,
            pick_outcome,
            remind_step,
            stop_run,
            list_sessions,
            get_session_messages,
            send_message,
            request_workflow,
            cancel_message,
            retry_message,
            stop_turn,
            answer_dialog,
            session_snapshot,
            session_terminal,
            restart_session,
            stop_session,
            remove_session,
            rename_session,
            archive_session,
            mark_session_seen,
            session_activity,
            session_glances,
            restore_session,
            get_paused,
            set_paused,
            usage_limits,
            try_usage_limit_now,
            list_bot_threads,
            get_thread_messages,
            open_attachment,
            update_bot,
            delete_bot,
            open_bot_folder,
            open_bots_folder,
            open_logs_folder,
            open_project_folder,
            list_skills,
            create_skill,
            import_skill,
            export_config,
            export_workflow_config,
            workflows_in_config,
            preview_workflow_import,
            import_workflow_config,
            preview_config_import,
            import_config,
            get_skill_body,
            update_skill,
            delete_skill,
            open_skills_folder,
            get_settings,
            updates::set_automatic_updates,
            updates::check_startup_update,
            updates::get_update_state,
            updates::download_update,
            updates::install_update,
            updates::dismiss_update,
            updates::open_update_link,
            updates::restart_after_update_failure,
            set_hop_limit,
            set_language,
            set_browser_shown,
            set_bots_root,
            set_project_folder,
            finish_setup,
            detect_providers,
            herdr_status,
            browser_open,
            browser_place,
            browser_close,
            browser_navigate,
            browser_show,
            browser_back,
            browser_forward,
            browser_reload,
            browser_pick_start,
            browser_pick_stop,
            browser_pick_target,
            browser_pick_result,
            browser_pick_box_state,
            browser_pick_submit,
            browser_pick_close,
            omarchy_theme
        ])
        .setup(move |app| {
            if let Some(watch) = omarchy_theme::watch(app.handle().clone()) {
                app.manage(watch);
            }
            if let Err(e) = tray::build(app.handle()) {
                tracing::warn!(error = %e, "could not add the menu-bar item; the Dock icon still opens the window");
            }
            let env = app.state::<AppEnv>();
            let (cell, shell) = (env.user.clone(), env.shell.clone());
            tauri::async_runtime::spawn(async move {
                cell.get_or_init(|| user_env(shell)).await;
            });
            let signalled = Arc::new(AtomicBool::new(false));
            for kind in [SignalKind::terminate(), SignalKind::interrupt(), SignalKind::hangup()] {
                let (handle, signalled) = (app.handle().clone(), signalled.clone());
                tauri::async_runtime::spawn(async move {
                    match tokio::signal::unix::signal(kind) {
                        Ok(mut signal) => {
                            while signal.recv().await.is_some() {
                                if signalled.swap(true, Ordering::SeqCst) {
                                    tracing::warn!(?kind, "second signal; exiting now");
                                    std::process::exit(130);
                                }
                                tracing::info!(?kind, "signal received; quitting");
                                handle.exit(0);
                            }
                        }
                        Err(e) => tracing::warn!(?kind, error = %e, "could not listen for the signal"),
                    }
                });
            }
            let handle = app.handle().clone();
            let engine = for_setup.clone();
            tauri::async_runtime::spawn(async move {
                let mut rx = engine.subscribe_ui();
                let recovering = engine.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = recovering.recover().await {
                        tracing::error!(error = %e, "could not recover the previous run's messages");
                    }
                    recovering.run_scheduler(SCHEDULER_EVERY);
                });
                loop {
                    match rx.recv().await {
                        Ok(event) => {
                            if let UiEvent::Paused { paused } = &event {
                                tray::set_paused(&handle, *paused);
                            }
                            let in_front = || window_in_front(&handle);
                            if let Some(mut notice) = notify::notice_for(&event, in_front, |id| {
                                notify::session_name(id, |id| engine.session_with_bot(id))
                            }) {
                                if let Some(id) = engine.event_project(&event) {
                                    if let Ok(projects) = engine.list_projects() {
                                        if let Some(project) = projects.iter().find(|p| p.id == id) {
                                            notice.title = format!("{} · {}", project.name, notice.title);
                                        }
                                    }
                                }
                                notify::send(&handle, &notice);
                            }
                            let mut payload = serde_json::to_value(&event).unwrap_or_default();
                            if let Some(project) = engine.event_project(&event) {
                                payload["project_id"] = project.into();
                            }
                            let _ = handle.emit("ui-event", payload);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            });
            Ok(())
        })
        .build(context)
        .expect("error while building Sonata")
        .run(move |_app, event| match event {
            RunEvent::ExitRequested { api, code, .. } => {
                if _app.state::<updates::Updates>().prevents_exit(code) {
                    api.prevent_exit();
                }
            }
            RunEvent::Exit => {
                if let Some(watch) = _app.try_state::<omarchy_theme::Watch>() {
                    watch.stop();
                }
                tauri::async_runtime::block_on(for_exit.shutdown());
            }
            #[cfg(target_os = "macos")]
            RunEvent::Reopen { has_visible_windows: false, .. } => tray::show_window(_app),
            _ => {}
        });
}
