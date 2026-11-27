import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { onOmarchyTheme, readOmarchyTheme, type OmarchyPalette } from "./omarchyTheme";

export type { OmarchyPalette };

export type Provider = "claude" | "codex" | "cursor" | "grok" | "opencode";
export type BotStatus = "offline" | "starting" | "idle" | "working" | "needs_attention" | "limited";
export type MessageKind = "text" | "raw_fallback" | "system";
export type MessageStatus = "queued" | "delivered" | "cancelled" | "interrupted" | "failed" | "retried";
export type DialogAnswer = "continue" | "cancel";

export interface Project {
  id: string;
  name: string;
  default_directory: string | null;
  default_provider: Provider;
  master_id: string | null;
  archived_at: string | null;
  created_at: string;
  updated_at: string;
}
export interface ProjectInput {
  name: string;
  default_directory: string | null;
  default_provider: Provider;
}

export interface Bot {
  project_id: string;
  id: string;
  handle: string;
  display_name: string;
  color: string;
  provider: Provider;
  model: string | null;
  instructions: string;
  config_version: number;
  skill_ids: string[];
  created_at: string;
  updated_at: string;
}

export interface BotView extends Bot {
  status: BotStatus;
}

export interface CardLine {
  text: string;
  at: string;
}

export interface SentLine {
  recipient: string;
  text: string;
  at: string;
}

export interface CutOff {
  message_id: string;
  sender: string;
  at: string;
}

export interface Reply {
  message_id: string;
  text: string;
  files: string[];
  at: string;
  raw: boolean;
}

export interface SessionActivity {
  session_id: string;
  working_on: CardLine | null;
  queued: number;
  question: string | null;
  last_sent: SentLine | null;
  cut_off: CutOff | null;
  last_reply: Reply | null;
  last_for_user_at: string | null;
  run_of: string | null;
  workflow: RunGlance | null;
}

export interface Glance {
  session_id: string;
  step: string | null;
  background_work: boolean;
  tail: string | null;
}

export interface SessionView {
  project_id: string;
  id: string;
  bot_id: string;
  number: number;
  title: string | null;
  provider_session_id: string | null;
  session_config_version: number | null;
  opened_by: string | null;
  created_at: string;
  last_activity_at: string;
  archived_at: string | null;
  waiting_question_id: string | null;
  waiting_options: string[];
  waiting_since: string | null;
  open_request_id: string | null;
  seen_at: string | null;
  status: BotStatus;
}

export interface Attachment {
  name: string;
  path: string;
}

export interface Message {
  id: string;
  thread_id: string;
  sender: string;
  recipient: string;
  sender_session: string | null;
  recipient_session: string | null;
  kind: MessageKind;
  body: string;
  attachments: Attachment[];
  system_code: string | null;
  system_params: Record<string, string> | null;
  status: MessageStatus;
  chain_id: string;
  hop: number;
  in_reply_to: string | null;
  created_at: string;
  delivered_at: string | null;
  task_id: string | null;
  task_name: string | null;
  task_since: string | null;
  workflow_mark: WorkflowMark | null;
  workflow_request: boolean;
}

export interface ThreadSummary {
  id: string;
  participant_a: string;
  participant_b: string;
  session_a: string | null;
  session_b: string | null;
  last_message_at: string;
}

export interface SentMessage {
  session: SessionView;
  message: Message;
}

export interface UsageLimit {
  provider: Provider;
  resets_at: string | null;
}

export type OrchestrationTaskState = "waiting" | "running" | "awaiting_review" | "succeeded" | "failed" | "blocked" | "cancelled";
export interface OrchestrationTask {
  spec: { key: string; title: string; instructions: string; acceptance_criteria: string[];
    executor: { kind: "bot"; handle: string } | { kind: "workflow"; name: string }; depends_on: string[]; files: string[] };
  state: OrchestrationTaskState;
  session_id: string | null;
  workflow_run_id: string | null;
  request_message_id: string | null;
  result_message_id: string | null;
  result: string | null;
  attachments: Attachment[];
  review: string | null;
  archived_at: string | null;
}
export interface OrchestrationExecution extends OrchestrationTask { session_ids: string[] }
export interface Orchestration {
  id: string; project_id: string; session_id: string; request_message_id: string;
  title: string; goal: string; revision: number; status: "active" | "completed" | "cancelled";
  tasks: OrchestrationTask[]; executions: OrchestrationExecution[];
  final_message_id: string | null; final_report: string | null; final_files: Attachment[];
  created_at: string; ended_at: string | null;
}

export interface WorkItem {
  id: string; project_id: string; parent_id: string | null; title: string; instructions: string;
  state: "pending" | "working" | "needs_you" | "finished" | "done"; outcome: string | null;
  primary_session_id: string | null; session_ids: string[]; workflow_run_id: string | null;
  orchestration_id: string | null; execution: OrchestrationTask | null; manual_start: boolean;
  created_at: string; finished_at: string | null; archived_at: string | null;
  result_message_id: string | null; depends_on: string[];
}
export interface NewWorkItem {
  parent_id: string | null; title: string; instructions: string;
  executor: { kind: "bot"; handle: string } | { kind: "workflow"; name: string };
  manual_start: boolean; depends_on: string[];
}

export type UiEvent = { project_id?: string } & (
  | { type: "message_added"; message: Message }
  | { type: "message_updated"; message: Message }
  | { type: "bot_status"; bot_id: string; status: BotStatus }
  | { type: "session_status"; session_id: string; bot_id: string; status: BotStatus }
  | { type: "session_added"; session: SessionView }
  | { type: "session_updated"; session: SessionView }
  | { type: "session_removed"; session_id: string; bot_id: string; thread_ids: string[] }
  | { type: "paused"; paused: boolean }
  | { type: "usage_limits"; limits: UsageLimit[] }
  | { type: "projects_changed" }
  | { type: "skills_changed" }
  | { type: "bot_added"; bot: BotView }
  | { type: "bot_updated"; bot: BotView }
  | { type: "tasks_changed" }
  | { type: "workflows_changed" }
  | { type: "work_items_changed" }
  | { type: "orchestration_changed"; project_id: string; run_id: string }
  | {
      type: "workflow_run_changed";
      run_id: string;
      name: string;
      status: RunStatus;
      pause: PauseReason | null;
      step: string | null;
      session_ids: string[];
    }

);

export interface Task {
  project_id: string;
  id: string;
  name: string;
  bot_id: string;
  prompt: string;
  every_days: number;
  at: string;
  starts_on: string;
  enabled: boolean;
  next_run_at: string | null;
  last_run_at: string | null;
  last_run_session: string | null;
  skipped_at: string | null;
  created_at: string;
  updated_at: string;
}

export interface NewTask {
  name: string;
  bot_id: string;
  prompt: string;
  every_days: number;
  at: string;
  starts_on: string | null;
}

export interface WorkflowPos {
  x: number;
  y: number;
}

export interface WorkflowArrow {
  to: string;
  outcome: string | null;
}

export interface WorkflowStep {
  automatic_child_review?: boolean;
  id: string;
  name: string;
  bot_id: string;
  instructions: string;
  arrows: WorkflowArrow[];
  pos: WorkflowPos | null;
}

export interface WorkflowDef {
  version: number;
  start: string | null;
  steps: WorkflowStep[];
  start_pos: WorkflowPos | null;
  done_pos: WorkflowPos | null;
}

export interface Workflow {
  project_id: string;
  id: string;
  name: string;
  when_to_use: string;
  def: WorkflowDef;
  created_at: string;
  updated_at: string;
}

export interface NewWorkflow {
  name: string;
  when_to_use: string;
  def: WorkflowDef;
}

export type RunStatus = "running" | "paused" | "finished" | "stopped";

export type PauseReason =
  | { kind: "visit_cap"; step_id: string }
  | { kind: "no_result"; step_id: string }
  | { kind: "bot_missing"; step_id: string };

export interface WorkflowRun {
  project_id: string;
  id: string;
  workflow_id: string | null;
  name: string;
  def: WorkflowDef;
  request: string;
  request_files: string[];
  opener_session: string | null;
  root_session: string | null;
  status: RunStatus;
  pause: PauseReason | null;
  extra_visits: Record<string, number>;
  created_at: string;
  ended_at: string | null;
}

export interface Visit {
  id: string;
  run_id: string;
  ord: number;
  step_id: string;
  n: number;
  session_id: string | null;
  request_message: string;
  result_message: string | null;
  outcome: string | null;
  reminded: boolean;
  started_at: string;
  ended_at: string | null;
}

export interface RunView {
  run: WorkflowRun;
  visits: Visit[];
}

export interface RunGlance {
  run_id: string;
  name: string;
  status: RunStatus;
  step: string | null;
  handle: string | null;
  visit: number;
  pause: PauseReason | null;
  outcomes: string[];
  holder: string | null;
}

export type MarkRole = "request" | "reminder" | "result" | "finished";

export interface WorkflowMark {
  run_id: string;
  visit_id: string;
  workflow: string;
  step: string;
  visit: number;
  role: MarkRole;
  outcome: string | null;
}

export interface CreateBotInput {
  display_name: string;
  color: string;
  provider?: Provider | null;
  model: string | null;
  instructions: string;
  skill_ids: string[];
}

export type SkillSource = "written" | "imported";

export interface Skill {
  id: string;
  slug: string;
  name: string;
  description: string;
  source: SkillSource;
  created_at: string;
  updated_at: string;
}

export interface SkillView extends Skill {
  used_by: string[];
}

export interface SkillInput {
  name: string;
  description: string;
  body: string;
}

export const USER = "user";

export interface ProviderStatus {
  provider: Provider;
  path: string | null;
  version: string | null;
  logged_in: boolean | null;
}

export interface HerdrStatus {
  path: string;
  version: string | null;
  protocol: number | null;
  error: string | null;
}

export interface AppSettings {
  automatic_updates: boolean;
  hop_limit: number;
  bots_root: string;
  logs_dir: string;
  bots_exist: boolean;
  setup_done: boolean;
  onboarding_pending: boolean;
  language: string;
  language_resolved: string;
  language_system: string;
  project_folder: string | null;
}

export interface Placement {
  x: number;
  y: number;
  width: number;
  height: number;
  viewport_width: number;
  viewport_height: number;
}

export interface PageState {
  url: string;
  title: string;
  loading: boolean;
}

export interface PickedElement {
  url: string;
  title: string;
  selector: string;
  tag: string;
  text: string;
  html: string;
}

export interface BrowserPick {
  comment: string;
  element: PickedElement;
}

export interface PickStrings {
  hint: string;
  placeholder: string;
  keys: string;
  send: string;
  sending: string;
  failed: string;
  noTarget: string;
  tooLong: string;
}

export interface PickBoxState {
  target: string | null;
  strings: PickStrings | null;
  status: "idle" | "sending" | "failed";
  generation: number;
}

export type ImportMode = "add_new" | "add_and_replace";
export type ItemStatus =
  | { kind: "new" }
  | { kind: "unchanged" }
  | { kind: "different" }
  | { kind: "invalid"; code: string; params: Record<string, string> };
export interface PlannedItem {
  name: string;
  target: string;
  status: ItemStatus;
}
export interface ImportPreview {
  token: string;
  skills: PlannedItem[];
  bots: PlannedItem[];
  workflows: PlannedItem[];
}
export interface ExportSummary {
  bots: number;
  skills: number;
  workflows: number;
}
export interface ImportReport {
  written: { kind: "skill" | "bot" | "workflow"; name: string; replaced: boolean }[];
  backup: { file_name: string; kept_at: string | null } | null;
  error: { code: string; params: Record<string, string> } | null;
}
export type ImportOutcome = { outcome: "changed"; preview: ImportPreview } | { outcome: "done"; report: ImportReport };

export type DependencyChoice = "keep_local" | "replace";
export interface WorkflowImportRequest {
  workflow: string;
  workflow_choice: DependencyChoice | null;
  new_name: string | null;
  bots: Record<string, DependencyChoice>;
  skills: Record<string, DependencyChoice>;
}
export interface WorkflowFileEntry {
  name: string;
  when_to_use: string;
  steps: number;
}
export interface WorkflowImportItem extends PlannedItem {
  kind: "workflow" | "bot" | "skill";
  action: "add" | "replace" | "reuse" | "choose" | "blocked";
  can_keep: boolean;
  required_by: string[];
  changed_fields: string[];
  changed_files: string[];
  used_by_bots: string[];
  used_by_workflows: string[];
}
export interface WorkflowImportPreview {
  token: string;
  items: WorkflowImportItem[];
  can_import: boolean;
}
export type WorkflowImportOutcome =
  { outcome: "changed" | "blocked"; preview: WorkflowImportPreview } | { outcome: "done"; report: ImportReport };

export interface OnboardingProfile { job: string; tasks: string }
export interface OnboardingProgress {
  profile: OnboardingProfile;
  stage: "profile" | "workspace" | "suggestions" | "ready";
  project: ProjectInput | null;
  selected: string[];
  skip: boolean;
}
export interface OnboardingBot {
  handle: string; name: string; color: string; provider: Provider;
  model: string | null; checkout: boolean; master: boolean; skills: string[]; instructions: string;
}
export interface OnboardingSkill {
  slug: string; name: string; description: string;
  files: { path: string; text?: string; executable?: boolean; base64?: string }[];
}
export interface OnboardingWorkflow {
  name: string; when_to_use: string;
  definition: { version: number; start: string | null; steps: {
    id: string; name: string; bot: string | null; instructions: string;
    arrows: { to: string; outcome: string | null }[];
  }[] };
}
export interface OnboardingConfig {
  bots: OnboardingBot[]; skills: OnboardingSkill[]; workflows: OnboardingWorkflow[];
}
export interface OnboardingProposal extends Omit<OnboardingConfig, "workflows"> {
  workflows: (OnboardingWorkflow & { expected_output: string; requirements: string[] })[];
}
export interface OnboardingState {
  progress: OnboardingProgress;
  status: "idle" | "generating" | "ready" | "applying" | "partial" | "complete" | "failed" | "cancelled";
  generation: string | null; session_id: string | null; message_id: string | null;
  proposal: OnboardingProposal | null; bundle: OnboardingConfig | null;
  error: { code: string; params?: Record<string, string> } | null;
}
export interface OnboardingCreated { kind: "skill" | "bot" | "workflow"; id: string; name: string }
export interface OnboardingPreview {
  token: string; selected: string[]; config: OnboardingConfig; created: OnboardingCreated[];
}
export interface OnboardingApplyResult { state: OnboardingState; created: OnboardingCreated[]; changed: boolean }

export function projectApi(projectId?: string) {
  const invoke = <T>(command: string, args?: Record<string, unknown>): Promise<T> =>
    tauriInvoke<T>(command, { ...(projectId ? { projectId } : {}), ...args });
  return {
  getOnboarding: () => invoke<OnboardingState>("get_onboarding"),
  saveOnboarding: (progress: OnboardingProgress) => invoke<OnboardingState>("save_onboarding", { progress }),
  generateOnboarding: () => invoke<OnboardingState>("generate_onboarding"),
  cancelOnboarding: () => invoke<OnboardingState>("cancel_onboarding"),
  previewOnboarding: (selected: string[]) => invoke<OnboardingPreview>("preview_onboarding", { selected }),
  applyOnboarding: (selected: string[], token: string) => invoke<OnboardingApplyResult>("apply_onboarding", { selected, token }),
  finishOnboarding: () => invoke<void>("finish_onboarding"),
  listProjects: () => invoke<Project[]>("list_projects"),
  createProject: (input: ProjectInput) => invoke<Project>("create_project", { input }),
  updateProject: (projectId: string, input: ProjectInput) => invoke<Project>("update_project", { projectId, input }),
  archiveProject: (projectId: string) => invoke<Project>("archive_project", { projectId }),
  restoreProject: (projectId: string) => invoke<Project>("restore_project", { projectId }),
  finishProjectSetup: (input: ProjectInput) => invoke<Project>("finish_project_setup", { input }),
  listBots: () => invoke<BotView[]>("list_bots"),
  createBot: (input: CreateBotInput) => invoke<Bot>("create_bot", { input }),
  listSessions: () => invoke<SessionView[]>("list_sessions"),
  listWorkItems: () => invoke<WorkItem[]>("list_work_items"),
  createWorkItem: (input: NewWorkItem) => invoke<WorkItem>("create_work_item", { input }),
  startWorkItem: (taskId: string) => invoke<void>("start_work_item", { taskId }),
  archiveWorkItem: (taskId: string, archived: boolean) => invoke<void>("archive_work_item", { taskId, archived }),
  cancelWorkItem: (taskId: string) => invoke<void>("cancel_work_item", { taskId }),
  listOrchestrations: () => invoke<Orchestration[]>("list_orchestrations"),
  cancelOrchestration: (runId: string, task: string | null = null) => invoke<void>("cancel_orchestration", { runId, task }),
  archiveOrchestrationTask: (runId: string, task: string, archived: boolean) => invoke<void>("archive_orchestration_task", { runId, task, archived }),
  getSessionMessages: (sessionId: string) => invoke<Message[]>("get_session_messages", { sessionId }),
  sendMessage: (botId: string, sessionId: string | null, body: string, files: string[] = []) =>
    invoke<SentMessage>("send_message", { botId, sessionId, body, files }),
  cancelMessage: (messageId: string) => invoke<Message>("cancel_message", { messageId }),
  retryMessage: (messageId: string) => invoke<Message>("retry_message", { messageId }),
  stopTurn: (sessionId: string) => invoke<void>("stop_turn", { sessionId }),
  answerDialog: (sessionId: string, answer: DialogAnswer) => invoke<void>("answer_dialog", { sessionId, answer }),
  sessionSnapshot: (sessionId: string) => invoke<string | null>("session_snapshot", { sessionId }),
  sessionTerminal: (sessionId: string) => invoke<string | null>("session_terminal", { sessionId }),
  restartSession: (sessionId: string) => invoke<void>("restart_session", { sessionId }),
  stopSession: (sessionId: string) => invoke<void>("stop_session", { sessionId }),
  removeSession: (sessionId: string) => invoke<string[]>("remove_session", { sessionId }),
  renameSession: (sessionId: string, name: string) => invoke<SessionView>("rename_session", { sessionId, name }),
  sessionActivity: (sessionIds: string[]) => invoke<SessionActivity[]>("session_activity", { sessionIds }),
  sessionGlances: (sessionIds: string[]) => invoke<Glance[]>("session_glances", { sessionIds }),
  markSessionSeen: (sessionId: string) => invoke<void>("mark_session_seen", { sessionId }),
  archiveSession: (sessionId: string) => invoke<string[]>("archive_session", { sessionId }),
  restoreSession: (sessionId: string) => invoke<string[]>("restore_session", { sessionId }),
  getPaused: () => invoke<boolean>("get_paused"),
  setPaused: (paused: boolean) => invoke<void>("set_paused", { paused }),
  usageLimits: () => invoke<UsageLimit[]>("usage_limits"),
  tryUsageLimitNow: (provider: Provider) => invoke<void>("try_usage_limit_now", { provider }),
  listBotThreads: () => invoke<ThreadSummary[]>("list_bot_threads"),
  getThreadMessages: (threadId: string) => invoke<Message[]>("get_thread_messages", { threadId }),
  openAttachment: (messageId: string, index: number, reveal: boolean) =>
    invoke<void>("open_attachment", { messageId, index, reveal }),
  updateBot: (botId: string, input: CreateBotInput) => invoke<Bot>("update_bot", { botId, input }),
  deleteBot: (botId: string) => invoke<void>("delete_bot", { botId }),
  openBotFolder: (botId: string) => invoke<void>("open_bot_folder", { botId }),
  openBotsFolder: () => invoke<void>("open_bots_folder"),
  openLogsFolder: () => invoke<void>("open_logs_folder"),
  openProjectFolder: () => invoke<void>("open_project_folder"),
  listSkills: () => invoke<SkillView[]>("list_skills"),
  createSkill: (input: SkillInput) => invoke<Skill>("create_skill", { input }),
  importSkill: (path: string) => invoke<Skill>("import_skill", { path }),
  getSkillBody: (skillId: string) => invoke<string>("get_skill_body", { skillId }),
  updateSkill: (skillId: string, input: SkillInput) => invoke<Skill>("update_skill", { skillId, input }),
  deleteSkill: (skillId: string) => invoke<void>("delete_skill", { skillId }),
  openSkillsFolder: () => invoke<void>("open_skills_folder"),
  listTasks: () => invoke<Task[]>("list_tasks"),
  createTask: (input: NewTask) => invoke<Task>("create_task", { input }),
  updateTask: (taskId: string, input: NewTask) => invoke<Task>("update_task", { taskId, input }),
  setTaskEnabled: (taskId: string, enabled: boolean) => invoke<Task>("set_task_enabled", { taskId, enabled }),
  deleteTask: (taskId: string) => invoke<void>("delete_task", { taskId }),
  runTaskNow: (taskId: string) => invoke<SessionView>("run_task_now", { taskId }),

  listWorkflows: () => invoke<Workflow[]>("list_workflows"),
  createWorkflow: (input: NewWorkflow) => invoke<Workflow>("create_workflow", { input }),
  updateWorkflow: (workflowId: string, input: NewWorkflow) =>
    invoke<Workflow>("update_workflow", { workflowId, input }),
  deleteWorkflow: (workflowId: string) => invoke<void>("delete_workflow", { workflowId }),
  requestWorkflow: (body: string) => invoke<SentMessage>("request_workflow", { body }),
  startWorkflow: (workflowId: string, body: string, files: string[]) =>
    invoke<SessionView>("start_workflow", { workflowId, body, files }),
  getWorkflowRun: (runId: string) => invoke<RunView>("get_workflow_run", { runId }),
  anotherRound: (runId: string) => invoke<void>("another_round", { runId }),
  pickOutcome: (runId: string, outcome: string | null) => invoke<void>("pick_outcome", { runId, outcome }),
  remindStep: (runId: string) => invoke<void>("remind_step", { runId }),
  stopRun: (runId: string) => invoke<void>("stop_run", { runId }),
  getSettings: () => invoke<AppSettings>("get_settings"),
  setAutomaticUpdates: (enabled: boolean) => invoke<void>("set_automatic_updates", { enabled }),
  setHopLimit: (limit: number) => invoke<void>("set_hop_limit", { limit }),
  setLanguage: (choice: string) => invoke<string>("set_language", { choice }),
  setBrowserShown: (shown: boolean) => invoke<void>("set_browser_shown", { shown }),
  setBotsRoot: (path: string) => invoke<void>("set_bots_root", { path }),
  setProjectFolder: (path: string) => invoke<string[]>("set_project_folder", { path }),
  exportWorkflowConfig: (workflowId: string, path: string) =>
    invoke<ExportSummary>("export_workflow_config", { workflowId, path }),
  workflowsInConfig: (path: string) => invoke<WorkflowFileEntry[]>("workflows_in_config", { path }),
  previewWorkflowImport: (path: string, request: WorkflowImportRequest) =>
    invoke<WorkflowImportPreview>("preview_workflow_import", { path, request }),
  importWorkflowConfig: (path: string, request: WorkflowImportRequest, token: string) =>
    invoke<WorkflowImportOutcome>("import_workflow_config", { path, request, token }),
  exportConfig: (path: string) => invoke<ExportSummary>("export_config", { path }),
  previewConfigImport: (path: string) => invoke<ImportPreview>("preview_config_import", { path }),
  importConfig: (path: string, mode: ImportMode, token: string) =>
    invoke<ImportOutcome>("import_config", { path, mode, token }),
  finishSetup: (projectFolder: string | null) => invoke<void>("finish_setup", { projectFolder }),
  detectProviders: () => invoke<ProviderStatus[]>("detect_providers"),
  herdrStatus: () => invoke<HerdrStatus>("herdr_status"),
  omarchyTheme: () => readOmarchyTheme(),
  onOmarchyTheme: (handler: (palette: OmarchyPalette | null) => void) => onOmarchyTheme(handler),
  onOpenSettings: (handler: () => void): Promise<UnlistenFn> => listen("open-settings", () => handler()),
  onOpenAbout: (handler: () => void): Promise<UnlistenFn> => listen("open-about", () => handler()),
  onToggleBrowser: (handler: () => void): Promise<UnlistenFn> => listen("toggle-browser", () => handler()),
  browserOpen: (url: string, rect: Placement) => invoke<PageState>("browser_open", { url, rect }),
  browserNavigate: (address: string, rect: Placement) => invoke<void>("browser_navigate", { address, rect }),
  browserShow: (rect: Placement) => invoke<PageState | null>("browser_show", { rect }),
  browserBack: () => invoke<void>("browser_back"),
  browserForward: () => invoke<void>("browser_forward"),
  browserReload: () => invoke<void>("browser_reload"),
  browserPlace: (rect: Placement) => invoke<void>("browser_place", { rect }),
  browserClose: () => invoke<void>("browser_close"),
  onBrowserState: (handler: (page: PageState) => void): Promise<UnlistenFn> =>
    listen<PageState>("browser-state", (e) => handler(e.payload)),
  browserPickStart: (strings: PickStrings, target: string | null) =>
    invoke<void>("browser_pick_start", { strings, target }),
  browserPickStop: () => invoke<void>("browser_pick_stop"),
  browserPickTarget: (target: string | null) => invoke<void>("browser_pick_target", { target }),
  browserPickResult: (ok: boolean) => invoke<void>("browser_pick_result", { ok }),
  onBrowserPick: (handler: (pick: BrowserPick) => void): Promise<UnlistenFn> =>
    listen<BrowserPick>("browser-pick", (e) => handler(e.payload)),
  onBrowserPickMode: (handler: (active: boolean) => void): Promise<UnlistenFn> =>
    listen<{ active: boolean }>("browser-pick-mode", (e) => handler(e.payload.active)),
  onTogglePick: (handler: () => void): Promise<UnlistenFn> => listen("toggle-pick", () => handler()),
  onBrowserPickRefused: (handler: () => void): Promise<UnlistenFn> => listen("browser-pick-refused", () => handler()),
  onUiEvent: (handler: (event: UiEvent) => void): Promise<UnlistenFn> =>
    listen<UiEvent>("ui-event", (e) => handler(e.payload)),
};

}
export const api = projectApi();
