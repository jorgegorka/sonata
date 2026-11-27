// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type { BotView, SessionView, WorkItem } from "../api";
import { taskExecution, taskProgress } from "../workItems";
import i18n from "../i18n";
import WorkBoard from "./WorkBoard";
const calls = vi.hoisted(() => [] as [string, unknown][]);
vi.mock("@tauri-apps/api/core", () => ({ invoke: (name: string, args: unknown) => { calls.push([name,args]); return Promise.resolve([]); } }));
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const bot: BotView = { id:"b", project_id:"default", handle:"worker", display_name:"Worker", color:"#456789", provider:"claude", model:null, instructions:"", config_version:1, skill_ids:[], created_at:"t", updated_at:"t", status:"idle" };
const session = (id: string): SessionView => ({ id,project_id:"default",bot_id:"b",number:1,title:id,provider_session_id:null,session_config_version:null,opened_by:null,created_at:"2026-11-27T10:00:00Z",last_activity_at:"2026-11-27T10:00:00Z",archived_at:null,waiting_question_id:null,waiting_options:[],waiting_since:null,open_request_id:null,seen_at:null,status:"idle" });
const item = (id: string, extra: Partial<WorkItem> = {}): WorkItem => ({ id,project_id:"default",parent_id:null,title:id,instructions:"Do the work",state:"working",outcome:null,primary_session_id:id,session_ids:[id],workflow_run_id:null,orchestration_id:null,execution:null,manual_start:false,created_at:"2026-11-27T10:00:00Z",finished_at:null,archived_at:null,result_message_id:null,depends_on:[],...extra });
const data = { activity:{},glances:{},attentionSince:{} };
let root: Root | undefined, host: HTMLDivElement;
beforeAll(() => i18n.changeLanguage("en"));
afterEach(async () => { if (root) await act(async () => root!.unmount()); root=undefined; host?.remove(); calls.length=0; });
async function mount(items: WorkItem[]) {
  host=document.createElement("div"); document.body.append(host); root=createRoot(host);
  await act(async () => root!.render(<WorkBoard items={items} bots={[bot]} sessions={items.flatMap((w) => w.session_ids.map(session))} paused={false} limits={[]} onTogglePause={() => {}} onOpen={() => {}} onNewTask={() => {}} onError={(e) => { throw e; }} data={data} now={new Date("2026-11-27T11:00:00Z")} />));
}
const button = (text: string) => [...host.querySelectorAll("button")].find((b) => b.textContent === text)!;
describe("unified tasks", () => {
  it("does not finish an idle conversation without an explicit result", () => {
    const execution=taskExecution(item("parent"),{sessions:[session("parent")],...data,paused:false,limits:[],providers:{b:"claude"}});
    expect(execution?.column).toBe("running");
  });
  it("counts only direct children, including Done, and distinguishes unsuccessful outcomes", () => {
    const items=[item("parent"),item("a",{parent_id:"parent",state:"finished"}),item("b",{parent_id:"parent",state:"done",outcome:"failed"}),item("grandchild",{parent_id:"a",state:"finished"})];
    expect(taskProgress("parent",items)).toEqual({total:2,finished:2,unsuccessful:1});
  });
  it("hides Pending by default but keeps its count and parent progress visible", async () => {
    await mount([item("parent"),item("child",{parent_id:"parent",state:"pending",primary_session_id:null,session_ids:[],manual_start:true})]);
    expect(host.querySelector('[data-column="pending"]')).toBeNull();
    expect(button("Show Pending (1)")).toBeTruthy();
    expect(host.querySelector("progress")?.max).toBe(1);
    expect(host.querySelector("progress")?.value).toBe(0);
    await act(async () => button("Show Pending (1)").click());
    expect(host.querySelector('[data-column="pending"]')?.textContent).toContain("Held for manual start");
    await act(async () => button("Start").click());
    expect(calls).toContainEqual(["start_work_item",{taskId:"child"}]);
  });
  it("renders parents and children as separate cards with the same status behavior", async () => {
    await mount([item("parent"),item("child",{parent_id:"parent",state:"finished",finished_at:"2026-11-27T10:30:00Z"})]);
    expect(host.querySelectorAll('[data-column="running"] article')).toHaveLength(1);
    expect(host.querySelectorAll('[data-column="finished"] article')).toHaveLength(1);
    expect(button("Parent: parent")).toBeTruthy();
    await act(async () => button("Mark done").click());
    expect(calls).toContainEqual(["archive_work_item",{taskId:"child",archived:true}]);
  });
});
