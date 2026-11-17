import { useCallback, useEffect, useReducer, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import {
  api,
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
  const [state, dispatch] = useReducer(reducer, initialState);
  const [browser, dispatchBrowser] = useReducer(browserReducer, panelStorage(), initialBrowser);
  const [page, setPage] = useState<Page | null>(null);
  const [drawer, setDrawer] = useState<DrawerView | null>(null);
  const [archiving, setArchiving] = useState<string | null>(null);
  const pageRef = useRef(page);
  pageRef.current = page;
  const drawerRef = useRef(drawer);
  drawerRef.current = drawer;
  const [error, setError] = useState<string | null>(null);
  const [tasks, setTasks] = useState<Task[]>([]);
  const [workflows, setWorkflows] = useState<Workflow[]>([]);
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
  const refreshTasks = useCallback(() => void api.listTasks().then(setTasks).catch(fail), [fail]);
  const refreshWorkflows = useCallback(() => void api.listWorkflows().then(setWorkflows).catch(fail), [fail]);
  const followFirstSend = useCallback((created: SessionView, focus = true) => {
    dispatch({ type: "ui_event", event: { type: "session_added", session: created } });
    const inDrawer = followsInDrawer(pageRef.current?.kind === "board" ? drawerRef.current : null, created);
    if (!inDrawer && !followsFirstSend(pageRef.current, created)) return;
    setDrafts((all) => moveDraft(all, created.bot_id, created.id));
    if (focus) setFollowed(created.id);
    if (inDrawer) setDrawer({ root: created.id, show: { kind: "session", id: created.id } });
    else setPage({ kind: "session", id: created.id });
  }, []);
  const followRunStart = useCallback((root: SessionView, workflowId: string) => {
    dispatch({ type: "ui_event", event: { type: "session_added", session: root } });
    if (pageRef.current?.kind !== "board" || !followsRunStart(drawerRef.current, workflowId)) return;
    setFollowed(root.id);
    setDrawer({ root: root.id, show: { kind: "session", id: root.id } });
  }, []);

  const followWorkflowRequest = useCallback((sent: SentMessage) => {
    dispatch({ type: "ui_event", event: { type: "session_added", session: sent.session } });
    dispatch({ type: "ui_event", event: { type: "message_added", message: sent.message } });
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
  }, [fail, checkProviders, loadBots, refreshSkills, refreshTasks, refreshWorkflows, togglePick]);

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
            refreshSkills();
            loadBots(false);
          }}
        />
      </main>
    );
  }

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
    setPage(full);
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
          if (!inDrawer) setPage(home());
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
            setPage(home());
          } finally {
            setArchiving(null);
          }
        }}
        focusComposer={chatSession === null || chatSession.id === followed}
        messages={chatSession ? (state.messages[chatSession.id] ?? []) : []}
        onError={fail}
        onEdit={() => setPage({ kind: "edit", id: chatBot.id })}
        draft={drafts[key] ?? EMPTY_DRAFT}
        onDraftChange={(update) => updateDraft(key, update)}
        browserOpen={inDrawer ? false : browser.open}
        onToggleBrowser={inDrawer ? drawerBrowser : toggleBrowser}
        onBack={inDrawer ? undefined : () => setPage({ kind: "board" })}
      />
    );
  };

  const drawerOver = (view: DrawerView) => (boardTasks: BoardTask[], now: Date, data: BoardData) => {
    const task = view.root === null ? null : (boardTasks.find((candidate) => candidate.id === view.root) ?? null);
    if (view.root !== null && task === null) return null;
    const show = view.show;
    let body: ReactNode = null;
    if (show.kind === "handoffs" && task) {
      const live = state.threads.filter((th) => !hasArchivedEnd(th, state.sessions));
      body = (
        <HandoffList
          threads={handoffsOf(task.members, live)}
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
        />
      );
    } else if (show.kind === "session") {
      const shown = state.sessions.find((s) => s.id === show.id);
      const shownBot = shown && state.bots.find((b) => b.id === shown.bot_id);
      body = shown && shownBot && chatOf(shownBot, shown, true);
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
          onBoard={() => setPage({ kind: "board" })}
          onBot={(botId) => {
            setPage({ kind: "board" });
            setDrawer(blankChat(botId));
          }}
          onNewBot={() => setPage({ kind: "new" })}
          onWorkflows={() => setPage({ kind: "workflows" })}
          onTasks={() => setPage({ kind: "tasks" })}
          onArchive={() => setPage({ kind: "archive" })}
          onSettings={() => setPage({ kind: "settings", tab: "settings" })}
        />
        <main className="main">
          {banner}
          {page?.kind === "board" ? (
            <Board
              bots={state.bots}
              sessions={state.sessions}
              paused={state.paused}
              limits={state.usageLimits}
              onTogglePause={() => void api.setPaused(!state.paused).catch(fail)}
              onOpen={(task) => setDrawer(openTask(task))}
              onNewTask={() =>
                state.bots.length > 0 ? setDrawer(blankChat(state.bots[0].id)) : setPage({ kind: "new" })
              }
              onError={fail}
              drawer={drawer ? drawerOver(drawer) : undefined}
            />
          ) : page?.kind === "new" ? (
            <BotForm
              onError={fail}
              providers={providers}
              skills={state.skills}
              onCancel={state.bots.length > 0 ? () => setPage(home()) : undefined}
              onSaved={(created) => {
                dispatch({ type: "bot_created", bot: created });
                refreshSkills();
                setPage({ kind: "newSession", botId: created.id });
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
              onCancel={() => setPage(home())}
              onSaved={(saved) => {
                dispatch({ type: "bot_updated", bot: saved });
                refreshSkills();
                setPage(home());
              }}
              onDeleted={(botId) => {
                dispatch({ type: "bot_deleted", botId });
                refreshSkills();
                refreshTasks();
                updateDraft(newSessionKey(botId), () => EMPTY_DRAFT);
                for (const s of state.sessions) if (s.bot_id === botId) updateDraft(s.id, () => EMPTY_DRAFT);
                setPage(home(state.bots.filter((b) => b.id !== botId)));
              }}
            />
          ) : page?.kind === "workflows" ? (
            <WorkflowsView
              workflows={workflows}
              bots={state.bots}
              onNew={() => setPage({ kind: "newWorkflow" })}
              onEdit={(id) => setPage({ kind: "editWorkflow", id })}
              onRun={(id) => {
                setPage({ kind: "board" });
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
              onCancel={() => setPage({ kind: "workflows" })}
              onSaved={() => {
                refreshWorkflows();
                setPage({ kind: "workflows" });
              }}
            />
          ) : editingWorkflow ? (
            <WorkflowEditor
              key={editingWorkflow.id}
              workflow={editingWorkflow}
              bots={state.bots}
              otherNames={workflows.filter((w) => w.id !== editingWorkflow.id).map((w) => w.name)}
              onCancel={() => setPage({ kind: "workflows" })}
              onSaved={() => {
                refreshWorkflows();
                setPage({ kind: "workflows" });
              }}
            />
          ) : page?.kind === "tasks" ? (
            <TasksView
              tasks={tasks}
              bots={state.bots}
              sessions={state.sessions}
              onNew={() => setPage({ kind: "newTask" })}
              onEdit={(id) => setPage({ kind: "editTask", id })}
              onOpenSession={(id) => setPage({ kind: "session", id })}
              onError={fail}
            />
          ) : page?.kind === "newTask" ? (
            <TaskForm
              bots={state.bots}
              onCancel={() => setPage({ kind: "tasks" })}
              onSaved={() => {
                refreshTasks();
                setPage({ kind: "tasks" });
              }}
            />
          ) : editingTask ? (
            <TaskForm
              key={editingTask.id}
              task={editingTask}
              bots={state.bots}
              onCancel={() => setPage({ kind: "tasks" })}
              onSaved={() => {
                refreshTasks();
                setPage({ kind: "tasks" });
              }}
            />
          ) : page?.kind === "newSkill" ? (
            <SkillForm
              onError={fail}
              onCancel={() => setPage(skillsTab)}
              onSaved={() => {
                refreshSkills();
                setPage(skillsTab);
              }}
            />
          ) : editingSkill ? (
            <SkillForm
              key={editingSkill.id}
              skill={editingSkill}
              onError={fail}
              onCancel={() => setPage(skillsTab)}
              onSaved={() => {
                refreshSkills();
                setPage(skillsTab);
              }}
              onDeleted={() => {
                refreshSkills();
                setPage(skillsTab);
              }}
            />
          ) : page?.kind === "archive" ? (
            <ArchiveView bots={state.bots} sessions={state.sessions} threads={state.threads} onOpen={setPage} />
          ) : page?.kind === "about" ? (
            <AboutView onError={fail} />
          ) : page?.kind === "settings" ? (
            <SettingsPage tab={page.tab} onTab={(tab) => setPage({ kind: "settings", tab })}>
              {page.tab === "skills" ? (
                <SkillsView
                  skills={state.skills}
                  onNew={() => setPage({ kind: "newSkill" })}
                  onOpen={(id) => setPage({ kind: "editSkill", id })}
                  onImported={(skill) => {
                    void refreshSkills().then((loaded) =>
                      setPage(loaded ? { kind: "editSkill", id: skill.id } : skillsTab),
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
                setPage({ kind: "archive" });
              }}
              onBack={() => setPage({ kind: "archive" })}
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
              onBackToArchive={archivedThread ? () => setPage({ kind: "archive" }) : undefined}
              onBack={archivedThread ? undefined : () => setPage({ kind: "board" })}
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
  );
}
