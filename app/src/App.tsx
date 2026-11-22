import ProjectsView from "./components/ProjectsView";
import { ProjectApiContext, ProjectContext } from "./projectContext";
import { useCallback, useEffect, useMemo, useReducer, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import {
  api, projectApi, type Project,
  type BotView,
  type BrowserPick,
  type PageState,
  type ProviderStatus,
  type SentMessage,
  type SessionView,
  type Task,
  type Workflow,
} from "./api";
import type { BoardTask } from "./board";
import AboutView from "./components/AboutView";
import ArchivedSessionView from "./components/ArchivedSessionView";
import ArchiveView from "./components/ArchiveView";
import Board, { markDone } from "./components/Board";
import BrowserPanel from "./components/BrowserPanel";
import Chat from "./components/Chat";
import BotForm from "./components/BotForm";
import Rail from "./components/Rail";
import SettingsPage, { type SettingsTab } from "./components/SettingsPage";
import SettingsView from "./components/SettingsView";
import SetupView from "./components/SetupView";
import SkillForm from "./components/SkillForm";
import SkillsView from "./components/SkillsView";
import TaskDrawer, { HandoffList } from "./components/TaskDrawer";
import TaskForm from "./components/TaskForm";
import TasksView from "./components/TasksView";
import ThreadView from "./components/ThreadView";
import WorkflowEditor from "./components/WorkflowEditor";
import WorkflowStart from "./components/WorkflowStart";
import WorkflowsView from "./components/WorkflowsView";
import type { BoardData } from "./components/useBoardData";
import {
  blankDrawer,
  chatShown,
  drawerAfter,
  followsInDrawer,
  followsRunStart,
  fullChatOf,
  handoffsOf,
  openTask,
  threadShown,
  type DrawerView,
} from "./drawer";
import { errorMessage, failUnless } from "./errors";
import { EMPTY_DRAFT, moveDraft, newSessionKey, withDraft, type Draft } from "./drafts";
import { followsFirstSend, isChat, type Selection } from "./selection";
import { browserReducer, gridColumns, initialBrowser, panelStorage, writeWidth } from "./browser";
import { reloadsSkills } from "./skills";
import { reloadsTasks } from "./tasks";
import { OpenLinkContext } from "./openLink";
import { PendingSessions, pickBody, pickStrings, pickTarget } from "./pick";
import i18n from "./i18n";
import {
  hasArchivedEnd,
  initialState,
  isArchived,
  reducer,
  removalCount,
  sessionLabel,
  showsArchived,
} from "./state";

type Page =
  | { kind: "board" }
  | { kind: "projects"; adding: boolean }
  | Selection
  | { kind: "new" }
  | { kind: "settings"; tab: SettingsTab }
  | { kind: "about" }
  | { kind: "edit"; id: string }
  | { kind: "archive" }
  | { kind: "workflows" }
  | { kind: "newWorkflow" }
  | { kind: "editWorkflow"; id: string }
  | { kind: "tasks" }
  | { kind: "newTask" }
  | { kind: "editTask"; id: string }
  | { kind: "newSkill" }
  | { kind: "editSkill"; id: string };

export default function App() {
  const { t } = useTranslation();
  const [allState, dispatch] = useReducer(reducer, initialState);
  const [projects, setProjects] = useState<Project[] | null>(null);
  const [projectId, setProjectId] = useState(() => { try { return localStorage.getItem("sonata.project") ?? "default"; } catch { return "default"; } });
  const projectRef = useRef(projectId);
  projectRef.current = projectId;
  const project = projects?.find((p) => p.id === projectId && !p.archived_at) ?? null;
  const scopedApi = useMemo(() => projectApi(projectId || undefined), [projectId]);
  const state = useMemo(() => {
    const belongs = (item: { project_id: string }) => item.project_id === projectId;
    const bots = allState.bots.filter(belongs);
    const sessions = allState.sessions.filter(belongs);
    const ids = new Set(sessions.map((session) => session.id));
    const threads = allState.threads.filter((thread) => (thread.session_a && ids.has(thread.session_a)) || (thread.session_b && ids.has(thread.session_b)));
    return { ...allState, bots, sessions, threads };
  }, [allState, projectId]);
  const [browser, dispatchBrowser] = useReducer(browserReducer, panelStorage(), initialBrowser);
  const [page, setPage] = useState<Page | null>(null);
  const [drawer, setDrawer] = useState<DrawerView | null>(null);
  const [archiving, setArchiving] = useState<string | null>(null);
  const pageRef = useRef(page);
  pageRef.current = page;
  const drawerRef = useRef(drawer);
  drawerRef.current = drawer;
  const [error, setError] = useState<string | null>(null);
  const [allTasks, setTasks] = useState<Task[]>([]);
  const [allWorkflows, setWorkflows] = useState<Workflow[]>([]);
  const tasks = allTasks.filter((task) => task.project_id === projectId);
  const workflows = allWorkflows.filter((workflow) => workflow.project_id === projectId);
  const [setupDone, setSetupDone] = useState<boolean | null>(null);
  const [providers, setProviders] = useState<ProviderStatus[] | null>(null);
  const tRef = useRef(t);
  tRef.current = t;
  const fail = useCallback((e: unknown) => setError(errorMessage(tRef.current, e)), []);
  const openInline = useCallback((url: string) => dispatchBrowser({ type: "open", url }), []);
  const browserPage = useCallback((page: PageState) => dispatchBrowser({ type: "page", page }), []);
  const closeBrowser = useCallback(() => dispatchBrowser({ type: "close" }), []);
  const toggleBrowser = useCallback(() => dispatchBrowser({ type: "toggle" }), []);
  const panelShown = browser.open && isChat(page);
  useEffect(() => {
    if (panelShown) return;
    api.browserClose().catch(fail);
    dispatchBrowser({ type: "hide" });
  }, [panelShown, fail]);
  useEffect(() => void api.setBrowserShown(browser.open).catch(fail), [browser.open, fail]);
  const stallBrowser = useCallback(() => dispatchBrowser({ type: "stalled" }), []);
  const resizeBrowser = useCallback(
    (width: number) => dispatchBrowser({ type: "resize", width, windowWidth: window.innerWidth }),
    [],
  );
  useEffect(() => writeWidth(panelStorage(), browser.width), [browser.width]);
  const [drafts, setDrafts] = useState<Record<string, Draft>>({});
  const [followed, setFollowed] = useState<string | null>(null);
  const updateDraft = useCallback(
    (key: string, update: (draft: Draft) => Draft) =>
      setDrafts((all) => withDraft(all, key, update(all[key] ?? EMPTY_DRAFT))),
    [],
  );
  const checkProviders = useCallback(() => {
    setProviders(null);
    api.detectProviders().then(setProviders).catch(fail);
  }, [fail]);
  const loadBots = useCallback(
    (keepPage: boolean) =>
      Promise.all([api.listBots(), api.listSessions()])
        .then(([bots, sessions]) => {
          dispatch({ type: "bots_loaded", bots });
          dispatch({ type: "sessions_loaded", sessions });
          const first: Page = bots.length === 0 ? { kind: "new" } : { kind: "board" };
          setPage((current) => (keepPage && current) || first);
        })
        .catch(fail),
    [fail],
  );
  const refreshSkills = useCallback(
    () =>
      api
        .listSkills()
        .then((skills) => {
          dispatch({ type: "skills_loaded", skills });
          return true;
        })
        .catch((e) => {
          fail(e);
          return false;
        }),
    [fail],
  );
  const refreshProjects = useCallback((select?: string) => {
    return api.listProjects().then((items) => {
      setProjects(items);
      setProjectId((current) => {
        const wanted = select ?? current;
        return items.some((p) => p.id === wanted && !p.archived_at) ? wanted : items.find((p) => !p.archived_at)?.id ?? "";
      });
    }).catch(fail);
  }, [fail]);
  useEffect(() => {
    try { localStorage.setItem("sonata.project", projectId); } catch { /* storage can be unavailable */ }
    setPage({ kind: "board" });
    setDrawer(null);
    setFollowed(null);
    dispatchBrowser({ type: "close" });
    void api.browserPickStop().catch(fail);
  }, [projectId, fail]);
  const refreshTasks = useCallback(() => void api.listTasks().then(setTasks).catch(fail), [fail]);
  const refreshWorkflows = useCallback(() => void api.listWorkflows().then(setWorkflows).catch(fail), [fail]);
  const followFirstSend = useCallback((created: SessionView, focus = true) => {
    dispatch({ type: "ui_event", event: { type: "session_added", session: created } });
    if (created.project_id !== projectRef.current) return;
    const inDrawer = followsInDrawer(pageRef.current?.kind === "board" ? drawerRef.current : null, created);
    if (!inDrawer && !followsFirstSend(pageRef.current, created)) return;
    setDrafts((all) => moveDraft(all, created.bot_id, created.id));
    if (focus) setFollowed(created.id);
    if (inDrawer) setDrawer({ root: created.id, show: { kind: "session", id: created.id } });
    else setPage({ kind: "session", id: created.id });
  }, []);
  const followRunStart = useCallback((root: SessionView, workflowId: string) => {
    dispatch({ type: "ui_event", event: { type: "session_added", session: root } });
    if (root.project_id !== projectRef.current) return;
    if (pageRef.current?.kind !== "board" || !followsRunStart(drawerRef.current, workflowId)) return;
    setFollowed(root.id);
    setDrawer({ root: root.id, show: { kind: "session", id: root.id } });
  }, []);

  const followWorkflowRequest = useCallback((sent: SentMessage) => {
    dispatch({ type: "ui_event", event: { type: "session_added", session: sent.session } });
    dispatch({ type: "ui_event", event: { type: "message_added", message: sent.message } });
    if (sent.session.project_id !== projectRef.current) return;
    setFollowed(sent.session.id);
    setPage({ kind: "board" });
    setDrawer({ root: sent.session.id, show: { kind: "session", id: sent.session.id } });
  }, []);

  const browserRef = useRef(browser);
  browserRef.current = browser;
  const target = setupDone ? pickTarget(page, state.sessions, state.bots, archiving, t) : null;
  const targetRef = useRef(target);
  targetRef.current = target;
  const targetLabel = target ? t("pick.to", { target: target.label }) : null;
  const targetLabelRef = useRef(targetLabel);
  targetLabelRef.current = targetLabel;
  const startPick = useCallback(
    () => api.browserPickStart(pickStrings(tRef.current), targetLabelRef.current).catch(fail),
    [fail],
  );
  const stopPick = useCallback(() => api.browserPickStop().catch(fail), [fail]);
  useEffect(() => void api.browserPickTarget(targetLabel).catch(fail), [targetLabel, fail]);
  useEffect(() => {
    if (browserRef.current.picking) void startPick();
  }, [t, startPick]);
  const pendingSessions = useRef(new PendingSessions());
  const sendPick = useCallback(
    async (pick: BrowserPick) => {
      const to = targetRef.current;
      if (!to) {
        setError(tRef.current("browser.pickNoChat"));
        api.browserPickResult(false).catch(fail);
        return;
      }
      let ok = false;
      try {
        const pending = to.sessionId === null ? pendingSessions.current.get(to.botId) : undefined;
        const sessionId = to.sessionId ?? (await pending?.catch(() => null)) ?? null;
        const call = api.sendMessage(to.botId, sessionId, pickBody(pick));
        if (sessionId === null) void pendingSessions.current.track(to.botId, call.then((sent) => sent.session.id));
        const sent = await call;
        if (sessionId === null) followFirstSend(sent.session, false);
        ok = true;
      } catch (e) {
        fail(e);
      }
      api.browserPickResult(ok).catch(fail);
    },
    [fail, followFirstSend],
  );
  const sendPickRef = useRef(sendPick);
  sendPickRef.current = sendPick;
  const pendingPick = useRef(false);
  useEffect(() => {
    if (!browser.open) pendingPick.current = false;
  }, [browser.open]);
  const togglePick = useCallback(() => {
    const panel = browserRef.current;
    if (panel.picking) void stopPick();
    else if (!targetRef.current || !panel.url) return;
    else if (!panel.open) {
      pendingPick.current = true;
      dispatchBrowser({ type: "toggle" });
    } else void startPick();
  }, [startPick, stopPick]);
  const shownBrowser = useCallback(() => {
    if (!pendingPick.current) return;
    pendingPick.current = false;
    void startPick();
  }, [startPick]);
  const pickFromPanel = useCallback((on: boolean) => void (on ? startPick() : stopPick()), [startPick, stopPick]);

  useEffect(() => {
    let cancelled = false;
    const unlisteners: Array<() => void> = [];
    const keep = (unlisten: () => void) => {
      if (cancelled) unlisten();
      else unlisteners.push(unlisten);
    };
    api
      .onUiEvent((event) => {
        if (event.type === "projects_changed") { void refreshProjects(); void loadBots(true); refreshTasks(); refreshWorkflows(); }
        if (reloadsSkills(event)) refreshSkills();
        if (reloadsTasks(event)) refreshTasks();
        if (event.type === "workflows_changed") refreshWorkflows();
        dispatch({ type: "ui_event", event });
      })
      .then(keep)
      .catch(fail);
    api.onOpenSettings(() => setPage({ kind: "settings", tab: "settings" })).then(keep).catch(fail);
    api.onOpenAbout(() => setPage({ kind: "about" })).then(keep).catch(fail);
    api.onToggleBrowser(() => dispatchBrowser({ type: "toggle" })).then(keep).catch(fail);
    api.onBrowserState((page) => dispatchBrowser({ type: "page", page })).then(keep).catch(fail);
    api.onBrowserPick((pick) => void sendPickRef.current(pick)).then(keep).catch(fail);
    api.onBrowserPickMode((on) => dispatchBrowser({ type: "picking", on })).then(keep).catch(fail);
    api.onBrowserPickRefused(() => setError(tRef.current("browser.pickRefused"))).then(keep).catch(fail);
    api.onTogglePick(togglePick).then(keep).catch(fail);
    api
      .getSettings()
      .then(async (settings) => {
        await i18n.changeLanguage(settings.language_resolved);
        setSetupDone(settings.setup_done);
      })
      .catch(fail);
    void refreshProjects();
    loadBots(true);
    api.listBotThreads().then((threads) => dispatch({ type: "threads_loaded", threads })).catch(fail);
    api.getPaused().then((paused) => dispatch({ type: "paused_loaded", paused })).catch(fail);
    api.usageLimits().then((limits) => dispatch({ type: "limits_loaded", limits })).catch(fail);
    checkProviders();
    refreshSkills();
    refreshTasks();
    refreshWorkflows();
    return () => {
      cancelled = true;
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, [fail, checkProviders, loadBots, refreshSkills, refreshTasks, refreshWorkflows, refreshProjects, togglePick]);

  useEffect(() => {
    if (page?.kind !== "board") setDrawer(null);
  }, [page]);
  useEffect(
    () => setDrawer((current) => current && drawerAfter(current, state.sessions, state.threads)),
    [state.sessions, state.threads],
  );

  const shownChat = chatShown(page, drawer);
  const shownThread = threadShown(page, drawer);
  useEffect(() => {
    if (!shownChat) return;
    api
      .getSessionMessages(shownChat)
      .then((messages) => dispatch({ type: "messages_loaded", key: shownChat, messages }))
      .catch(fail);
  }, [shownChat, fail]);
  useEffect(() => {
    if (!shownThread) return;
    api
      .getThreadMessages(shownThread)
      .then((messages) => dispatch({ type: "messages_loaded", key: shownThread, messages }))
      .catch(fail);
  }, [shownThread, fail]);
  const seenChat =
    shownChat !== null && state.sessions.some((s) => s.id === shownChat && !isArchived(s)) ? shownChat : null;
  const seenCount = seenChat ? (state.messages[seenChat]?.length ?? 0) : 0;
  useEffect(() => {
    if (seenChat) api.markSessionSeen(seenChat).catch(failUnless("sessionNotFound", fail));
  }, [seenChat, seenCount, fail]);

  const banner = error && (
    <div className="error" role="alert" onClick={() => setError(null)}>
      {error}
    </div>
  );
  if (setupDone === null) return <main className="main">{banner}</main>;
  if (!setupDone) {
    return (
      <main className="main setup-page">
        {banner}
        <SetupView
          providers={providers}
          onCheckProviders={checkProviders}
          onError={fail}
          onDone={() => {
            setSetupDone(true);
            void refreshProjects("default");
            refreshSkills();
            loadBots(false);
          }}
        />
      </main>
    );
  }

  const navigate: typeof setPage = (next) => { if (projectRef.current === projectId) setPage(next); };
  const session = page?.kind === "session" ? (state.sessions.find((s) => s.id === page.id) ?? null) : null;
  const chatBotId = session ? session.bot_id : page?.kind === "newSession" ? page.botId : null;
  const bot = chatBotId ? (state.bots.find((b) => b.id === chatBotId) ?? null) : null;
  const editing = page?.kind === "edit" ? (state.bots.find((b) => b.id === page.id) ?? null) : null;
  const editingTask = page?.kind === "editTask" ? (tasks.find((task) => task.id === page.id) ?? null) : null;
  const editingWorkflow =
    page?.kind === "editWorkflow" ? (workflows.find((workflow) => workflow.id === page.id) ?? null) : null;
  const thread = page?.kind === "thread" ? (state.threads.find((th) => th.id === page.id) ?? null) : null;
  const editingSkill = page?.kind === "editSkill" ? (state.skills.find((s) => s.id === page.id) ?? null) : null;
  const archivedSession = showsArchived(session, archiving) ? session : null;
  const archivedThread = thread && hasArchivedEnd(thread, state.sessions) ? thread : null;
  const archiveOpen = page?.kind === "archive" || archivedSession !== null || archivedThread !== null;
  const home = (bots = state.bots): Page => (bots.length > 0 ? { kind: "board" } : { kind: "new" });
  const blankChat = (botId: string): DrawerView => blankDrawer({ kind: "bot", id: botId });
  const skillsTab: Page = { kind: "settings", tab: "skills" };

  const openFull = (view: DrawerView): boolean => {
    const full = fullChatOf(view);
    if (full === null) return false;
    navigate(full);
    setDrawer(null);
    return true;
  };
  const drawerLink = (url: string) => {
    if (drawer && openFull(drawer)) openInline(url);
  };
  const drawerBrowser = () => {
    if (drawer && openFull(drawer) && !browser.open) toggleBrowser();
  };

  const chatOf = (chatBot: BotView, chatSession: SessionView | null, inDrawer: boolean) => {
    const key = chatSession ? chatSession.id : newSessionKey(chatBot.id);
    return (
      <Chat
        key={chatSession?.id ?? `new:${chatBot.id}`}
        bot={chatBot}
        session={chatSession}
        label={chatSession ? sessionLabel(chatSession, state.sessions, state.bots, t) : null}
        cascade={chatSession ? removalCount(state.sessions, chatSession.id) : 0}
        onSent={(created) => followFirstSend(created)}
        onRemoved={(ids) => {
          for (const id of ids) updateDraft(id, () => EMPTY_DRAFT);
          if (!inDrawer) navigate(home());
        }}
        onArchive={async () => {
          if (!chatSession) return;
          if (inDrawer) {
            await api.archiveSession(chatSession.id);
            return;
          }
          setArchiving(chatSession.id);
          try {
            await api.archiveSession(chatSession.id);
            navigate(home());
          } finally {
            setArchiving(null);
          }
        }}
        focusComposer={chatSession === null || chatSession.id === followed}
        messages={chatSession ? (state.messages[chatSession.id] ?? []) : []}
        onError={fail}
        onEdit={() => navigate({ kind: "edit", id: chatBot.id })}
        draft={drafts[key] ?? EMPTY_DRAFT}
        onDraftChange={(update) => updateDraft(key, update)}
        browserOpen={inDrawer ? false : browser.open}
        onToggleBrowser={inDrawer ? drawerBrowser : toggleBrowser}
        onBack={inDrawer ? undefined : () => navigate({ kind: "board" })}
      />
    );
  };

  const drawerOver = (view: DrawerView) => (boardTasks: BoardTask[], now: Date, data: BoardData) => {
    const task = view.root === null ? null : (boardTasks.find((candidate) => candidate.id === view.root) ?? null);
    if (view.root !== null && task === null) return null;
    const show = view.show;
    let body: ReactNode = null;
    if (show.kind === "handoffs" && task) {
      const available = task.column === "done" ? state.threads : state.threads.filter((th) => !hasArchivedEnd(th, state.sessions));
      body = (
        <HandoffList
          threads={handoffsOf(task.members, available)}
          bots={state.bots}
          sessions={state.sessions}
          onOpen={(id) => setDrawer((current) => current && { ...current, show: { kind: "thread", id } })}
        />
      );
    } else if (show.kind === "thread") {
      const shown = state.threads.find((th) => th.id === show.id);
      body = shown && (
        <ThreadView
          key={shown.id}
          thread={shown}
          bots={state.bots}
          sessions={state.sessions}
          messages={state.messages[shown.id] ?? []}
          onError={fail}
          browserOpen={false}
          onToggleBrowser={drawerBrowser}
          readOnly={task?.column === "done"}
        />
      );
    } else if (show.kind === "session") {
      const shown = state.sessions.find((s) => s.id === show.id);
      const shownBot = shown && state.bots.find((b) => b.id === shown.bot_id);
      body = shown && shownBot && (isArchived(shown) ? (
        <ArchivedSessionView
          key={shown.id}
          embedded
          bot={shownBot}
          session={shown}
          label={sessionLabel(shown, state.sessions, state.bots, t)}
          cascade={removalCount(state.sessions, shown.id)}
          messages={state.messages[shown.id] ?? []}
          onRemoved={(ids) => {
            for (const id of ids) updateDraft(id, () => EMPTY_DRAFT);
            setDrawer(null);
          }}
          onBack={() => setDrawer(null)}
          onError={fail}
          browserOpen={false}
          onToggleBrowser={drawerBrowser}
        />
      ) : chatOf(shownBot, shown, true));
    } else if (show.kind === "newSession") {
      const shownBot = state.bots.find((b) => b.id === show.botId);
      body = shownBot && chatOf(shownBot, null, true);
    } else if (show.kind === "newRun") {
      const workflow = workflows.find((w) => w.id === show.workflowId);
      body = workflow && (
        <WorkflowStart
          key={workflow.id}
          workflow={workflow}
          bots={state.bots}
          onStarted={(root) => followRunStart(root, workflow.id)}
          onError={fail}
        />
      );
    }
    return (
      <OpenLinkContext.Provider value={drawerLink}>
        <TaskDrawer
          task={task}
          drawer={view}
          bots={state.bots}
          workflows={workflows}
          sessions={state.sessions}
          activity={data.activity}
          now={now}
          onShow={(next) => setDrawer((current) => current && { ...current, show: next })}
          onPick={(target) => setDrawer(blankDrawer(target))}
          onOpenFull={() => void openFull(view)}
          onMarkDone={() => {
            if (task) {
              const name = sessionLabel(task.root, state.sessions, state.bots, t);
              void markDone(task, data.activity, data.glances, name, t).catch(fail);
            }
          }}
          onClose={() => setDrawer(null)}
          onError={fail}
        >
          {body}
        </TaskDrawer>
      </OpenLinkContext.Provider>
    );
  };

  return (
    <ProjectContext.Provider value={project}>
    <ProjectApiContext.Provider value={scopedApi}>
    <OpenLinkContext.Provider value={openInline}>
      <div className="app" style={panelShown ? { gridTemplateColumns: gridColumns(browser.width) } : undefined}>
        <Rail
          bots={state.bots}
          sessions={state.sessions}
          boardOpen={page?.kind === "board"}
          workflowsOpen={page?.kind === "workflows" || page?.kind === "newWorkflow" || page?.kind === "editWorkflow"}
          tasksOpen={page?.kind === "tasks" || page?.kind === "newTask" || page?.kind === "editTask"}
          archiveOpen={archiveOpen}
          settingsOpen={page?.kind === "settings" || page?.kind === "newSkill" || page?.kind === "editSkill"}
          onBoard={() => navigate({ kind: "board" })}
          onBot={(botId) => {
            navigate({ kind: "board" });
            setDrawer(blankChat(botId));
          }}
          onNewBot={() => navigate({ kind: "new" })}
          onWorkflows={() => navigate({ kind: "workflows" })}
          onTasks={() => navigate({ kind: "tasks" })}
          onArchive={() => navigate({ kind: "archive" })}
          onSettings={() => navigate({ kind: "settings", tab: "settings" })}
        />
        <main className="main" key={projectId}>
          <header className="project-header">
            <div className="project-switcher">
              {(projects?.filter((p) => !p.archived_at).length ?? 0) > 1 ? <label>
                <span className="sr-only">{t("projects.select")}</span>
                <select aria-label={t("projects.select")} value={projectId} onChange={(e) => setProjectId(e.target.value)}>
                  {projects?.filter((p) => !p.archived_at).map((p) => {
                    const attention = allState.sessions.some((session) => session.project_id === p.id && !session.archived_at && (session.waiting_since || session.status === "needs_attention"));
                    return <option key={p.id} value={p.id}>{p.name}{attention ? ` · ${t("sidebar.waitingForYou")}` : ""}</option>;
                  })}
                </select>
              </label> : <strong>{project?.name ?? t("projects.none")}</strong>}
            </div>
            <div className="header-actions">
              <button onClick={() => navigate({ kind: "projects", adding: true })}>{t("projects.add")}</button>
              <button onClick={() => navigate({ kind: "projects", adding: false })}>{t("projects.manage")}</button>
            </div>
          </header>
          {banner}
          {page?.kind === "projects" || (projects !== null && !project) ? (
            <ProjectsView key={page?.kind === "projects" ? String(page.adding) : "empty"} projects={projects ?? []} adding={page?.kind === "projects" && page.adding}
              onError={fail} onClose={() => navigate({ kind: "board" })}
              onChanged={(selected) => { void refreshProjects(projectRef.current === projectId ? selected : undefined); void loadBots(true); refreshTasks(); refreshWorkflows(); refreshSkills(); }} />
          ) : page?.kind === "board" ? (
            <Board
              bots={state.bots}
              sessions={state.sessions}
              paused={state.paused}
              limits={state.usageLimits}
              onTogglePause={() => void api.setPaused(!state.paused).catch(fail)}
              onOpen={(task) => setDrawer(openTask(task))}
              onNewTask={() =>
                state.bots.length > 0 ? setDrawer(blankChat(state.bots[0].id)) : navigate({ kind: "new" })
              }
              onError={fail}
              drawer={drawer ? drawerOver(drawer) : undefined}
            />
          ) : page?.kind === "new" ? (
            <BotForm
              onError={fail}
              providers={providers}
              skills={state.skills}
              onCancel={state.bots.length > 0 ? () => navigate(home()) : undefined}
              onSaved={(created) => {
                dispatch({ type: "bot_created", bot: created });
                refreshSkills();
                navigate({ kind: "newSession", botId: created.id });
              }}
            />
          ) : editing ? (
            <BotForm
              onError={fail}
              key={editing.id}
              bot={editing}
              providers={providers}
              skills={state.skills}
              taskCount={tasks.filter((task) => task.bot_id === editing.id).length}
              onCancel={() => navigate(home())}
              onSaved={(saved) => {
                dispatch({ type: "bot_updated", bot: saved });
                refreshSkills();
                navigate(home());
              }}
              onDeleted={(botId) => {
                dispatch({ type: "bot_deleted", botId });
                refreshSkills();
                refreshTasks();
                updateDraft(newSessionKey(botId), () => EMPTY_DRAFT);
                for (const s of state.sessions) if (s.bot_id === botId) updateDraft(s.id, () => EMPTY_DRAFT);
                navigate(home(state.bots.filter((b) => b.id !== botId)));
              }}
            />
          ) : page?.kind === "workflows" ? (
            <WorkflowsView
              workflows={workflows}
              bots={state.bots}
              onNew={() => navigate({ kind: "newWorkflow" })}
              onEdit={(id) => navigate({ kind: "editWorkflow", id })}
              onRun={(id) => {
                navigate({ kind: "board" });
                setDrawer(blankDrawer({ kind: "workflow", id }));
              }}
              onRequested={followWorkflowRequest}
              onImported={() => {
                void loadBots(true);
                void refreshSkills();
                void refreshWorkflows();
              }}
            />
          ) : page?.kind === "newWorkflow" ? (
            <WorkflowEditor
              workflow={null}
              bots={state.bots}
              otherNames={workflows.map((workflow) => workflow.name)}
              onCancel={() => navigate({ kind: "workflows" })}
              onSaved={() => {
                refreshWorkflows();
                navigate({ kind: "workflows" });
              }}
            />
          ) : editingWorkflow ? (
            <WorkflowEditor
              key={editingWorkflow.id}
              workflow={editingWorkflow}
              bots={state.bots}
              otherNames={workflows.filter((w) => w.id !== editingWorkflow.id).map((w) => w.name)}
              onCancel={() => navigate({ kind: "workflows" })}
              onSaved={() => {
                refreshWorkflows();
                navigate({ kind: "workflows" });
              }}
            />
          ) : page?.kind === "tasks" ? (
            <TasksView
              tasks={tasks}
              bots={state.bots}
              sessions={state.sessions}
              onNew={() => navigate({ kind: "newTask" })}
              onEdit={(id) => navigate({ kind: "editTask", id })}
              onOpenSession={(id) => navigate({ kind: "session", id })}
              onError={fail}
            />
          ) : page?.kind === "newTask" ? (
            <TaskForm
              bots={state.bots}
              onCancel={() => navigate({ kind: "tasks" })}
              onSaved={() => {
                refreshTasks();
                navigate({ kind: "tasks" });
              }}
            />
          ) : editingTask ? (
            <TaskForm
              key={editingTask.id}
              task={editingTask}
              bots={state.bots}
              onCancel={() => navigate({ kind: "tasks" })}
              onSaved={() => {
                refreshTasks();
                navigate({ kind: "tasks" });
              }}
            />
          ) : page?.kind === "newSkill" ? (
            <SkillForm
              onError={fail}
              onCancel={() => navigate(skillsTab)}
              onSaved={() => {
                refreshSkills();
                navigate(skillsTab);
              }}
            />
          ) : editingSkill ? (
            <SkillForm
              key={editingSkill.id}
              skill={editingSkill}
              onError={fail}
              onCancel={() => navigate(skillsTab)}
              onSaved={() => {
                refreshSkills();
                navigate(skillsTab);
              }}
              onDeleted={() => {
                refreshSkills();
                navigate(skillsTab);
              }}
            />
          ) : page?.kind === "archive" ? (
            <ArchiveView bots={state.bots} sessions={state.sessions} threads={state.threads} onOpen={setPage} />
          ) : page?.kind === "about" ? (
            <AboutView onError={fail} />
          ) : page?.kind === "settings" ? (
            <SettingsPage tab={page.tab} onTab={(tab) => navigate({ kind: "settings", tab })}>
              {page.tab === "skills" ? (
                <SkillsView
                  skills={state.skills}
                  onNew={() => navigate({ kind: "newSkill" })}
                  onOpen={(id) => navigate({ kind: "editSkill", id })}
                  onImported={(skill) => {
                    void refreshSkills().then((loaded) =>
                      navigate(loaded ? { kind: "editSkill", id: skill.id } : skillsTab),
                    );
                  }}
                  onError={fail}
                />
              ) : (
                <SettingsView
                  providers={providers}
                  onCheckProviders={checkProviders}
                  onBotsChanged={() => loadBots(true)}
                  onError={fail}
                />
              )}
            </SettingsPage>
          ) : archivedSession && bot ? (
            <ArchivedSessionView
              key={archivedSession.id}
              bot={bot}
              session={archivedSession}
              label={sessionLabel(archivedSession, state.sessions, state.bots, t)}
              cascade={removalCount(state.sessions, archivedSession.id)}
              messages={state.messages[archivedSession.id] ?? []}
              onRemoved={(ids) => {
                for (const id of ids) updateDraft(id, () => EMPTY_DRAFT);
                navigate({ kind: "archive" });
              }}
              onBack={() => navigate({ kind: "archive" })}
              onError={fail}
              browserOpen={browser.open}
              onToggleBrowser={toggleBrowser}
            />
          ) : bot ? (
            chatOf(bot, session, false)
          ) : thread ? (
            <ThreadView
              key={thread.id}
              thread={thread}
              bots={state.bots}
              sessions={state.sessions}
              messages={state.messages[thread.id] ?? []}
              onError={fail}
              browserOpen={browser.open}
              onToggleBrowser={toggleBrowser}
              onBackToArchive={archivedThread ? () => navigate({ kind: "archive" }) : undefined}
              onBack={archivedThread ? undefined : () => navigate({ kind: "board" })}
            />
          ) : null}
        </main>
        {panelShown && (
          <BrowserPanel
            state={browser}
            onPage={browserPage}
            onClose={closeBrowser}
            onStalled={stallBrowser}
            onResize={resizeBrowser}
            onError={fail}
            picking={browser.picking}
            pickAvailable={target !== null}
            onPick={pickFromPanel}
            onShown={shownBrowser}
          />
        )}
      </div>
    </OpenLinkContext.Provider>
    </ProjectApiContext.Provider>
    </ProjectContext.Provider>
  );
}
