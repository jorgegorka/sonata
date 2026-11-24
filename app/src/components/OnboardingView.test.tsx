// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { OnboardingState, OnboardingConfig, OnboardingProgress, ProviderStatus } from "../api";
import i18n from "../i18n";
import OnboardingView from "./OnboardingView";
const mocks = vi.hoisted(() => ({
  getOnboarding: vi.fn(), saveOnboarding: vi.fn(), getSettings: vi.fn(), listProjects: vi.fn(), listBots: vi.fn(),
  listSessions: vi.fn(), generateOnboarding: vi.fn(), cancelOnboarding: vi.fn(), previewOnboarding: vi.fn(),
  applyOnboarding: vi.fn(), finishOnboarding: vi.fn(), finishProjectSetup: vi.fn(), herdrStatus: vi.fn(),
  setBotsRoot: vi.fn(),
}));
vi.mock("../api", () => ({ api: mocks }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: () => Promise.resolve("/tmp/my-work") }));
vi.mock("./AttentionCard", () => ({ default: () => <p>Provider needs attention</p> }));
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
let stored: OnboardingState;
let root: Root;
let host: HTMLElement;
const done = vi.fn();
const provider: ProviderStatus = { provider:"claude", path:"/bin/claude", logged_in:true, version:"1" };
const config: OnboardingConfig = {
  workflows:["Weekly report","Short summary"].map(name => ({ name, when_to_use:"When comparing papers", definition:{version:1,start:"write",steps:[{id:"write",name:"Draft",bot:"writer",instructions:"Cite the supplied papers",arrows:[]}]}})),
  bots:[{handle:"writer",name:"Writer",color:"#4f46e5",provider:"claude",model:null,checkout:true,master:false,skills:["report"],instructions:"Write sourced reports"}],
  skills:[{slug:"report",name:"Reporting",description:"Report writing",files:[{path:"SKILL.md",text:"Use evidence"}]}],
};
function ready() {
  stored.status = "ready"; stored.generation = "generation"; stored.bundle = config;
  stored.proposal = { ...config, workflows:config.workflows.map(w => ({...w, expected_output:"A sourced document",requirements:["Papers"]})) };
  stored.progress.stage = "suggestions"; stored.progress.selected = config.workflows.map(w => w.name);
}
const button = (name: string) => [...host.querySelectorAll("button")].find(b => b.textContent === name)!;
const click = (el: Element) => act(async () => { el.dispatchEvent(new MouseEvent("click", {bubbles:true})); });
const input = (el: HTMLInputElement | HTMLTextAreaElement, value: string) => act(async () => {
  const proto = el.tagName === "TEXTAREA" ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(proto,"value")!.set!.call(el,value);
  el.dispatchEvent(new Event("input",{bubbles:true}));
});
async function mount(firstRun=true, providers=[provider]) {
  await act(async () => root.render(<OnboardingView firstRun={firstRun} providers={providers} onCheckProviders={vi.fn()} onDone={done} />));
}
describe("guided onboarding", () => {
  beforeAll(() => i18n.changeLanguage("en"));
  beforeEach(() => {
    vi.useFakeTimers(); Object.values(mocks).forEach(m => m.mockReset()); done.mockReset();
    stored={progress:{profile:{job:"",tasks:""},stage:"profile",project:null,selected:[],skip:false},status:"idle",generation:null,session_id:null,message_id:null,proposal:null,bundle:null,error:null};
    mocks.getOnboarding.mockImplementation(async () => structuredClone(stored));
    mocks.saveOnboarding.mockImplementation(async (progress: OnboardingProgress) => { stored.progress=structuredClone(progress); return structuredClone(stored); });
    mocks.getSettings.mockResolvedValue({setup_done:true,bots_root:"/tmp/bots",bots_exist:true});
    mocks.listProjects.mockResolvedValue([{id:"default",name:"Default",master_id:"master",default_provider:"claude"}]);
    mocks.listBots.mockResolvedValue([{id:"master",provider:"claude"}]);
    mocks.listSessions.mockResolvedValue([]);
    mocks.herdrStatus.mockResolvedValue({error:null});
    mocks.generateOnboarding.mockImplementation(async () => { stored.status="generating"; stored.session_id="session"; return structuredClone(stored); });
    mocks.cancelOnboarding.mockImplementation(async () => { stored.status="cancelled"; return structuredClone(stored); });
    mocks.previewOnboarding.mockImplementation(async (selected: string[]) => ({ token:selected.join(","),selected,config:{...config,workflows:config.workflows.filter(w=>selected.includes(w.name))},created:[] }));
    mocks.applyOnboarding.mockImplementation(async () => { stored.status="complete"; stored.progress.stage="ready"; return {state:structuredClone(stored),created:[],changed:false}; });
    mocks.finishOnboarding.mockResolvedValue(undefined);
    mocks.finishProjectSetup.mockResolvedValue({id:"default"});
    host=document.createElement("div");document.body.appendChild(host);root=createRoot(host);
  });
  afterEach(async () => { await act(async()=>root.unmount()); host.remove(); vi.useRealTimers(); });

  it("asks exactly two questions, saves answers and does not generate until requested", async () => {
    await mount();
    expect(host.querySelectorAll("form input,form textarea")).toHaveLength(2);
    expect(button("Continue").disabled).toBe(true);
    await input(host.querySelector("input")!,"Researcher");
    await input(host.querySelector("textarea")!,"Compare papers");
    await act(async()=>{await vi.advanceTimersByTimeAsync(350);});
    expect(stored.progress.profile).toEqual({job:"Researcher",tasks:"Compare papers"});
    await click(button("Continue"));
    expect(host.textContent).toContain("Build your starting team");
    expect(mocks.generateOnboarding).not.toHaveBeenCalled();
    await click(button("Suggest workflows"));
    expect(mocks.generateOnboarding).toHaveBeenCalledTimes(1);
    expect(host.textContent).toContain("preparing suggestions");
  });

  it("skipping on a new installation still completes required workspace setup", async () => {
    mocks.getSettings.mockResolvedValue({setup_done:false,bots_root:"/tmp/bots",bots_exist:false});
    await mount(); await click(button("Skip personalization"));
    expect(host.textContent).toContain("Choose where you work");
    expect(done).not.toHaveBeenCalled();
    await click(button("Choose…"));
    await click(button("Continue"));
    expect(mocks.finishProjectSetup).toHaveBeenCalledWith(expect.objectContaining({default_directory:"/tmp/my-work"}));
    expect(mocks.finishOnboarding).toHaveBeenCalledTimes(1);
    expect(done).toHaveBeenCalledWith(false);
    expect(mocks.generateOnboarding).not.toHaveBeenCalled();
  });

  it("creates only selected workflows with the reviewed token and shows the ready page", async () => {
    ready(); await mount();
    expect(host.querySelectorAll("article")).toHaveLength(2);
    expect(host.textContent).toContain("Use evidence");
    await click(host.querySelectorAll('input[type="checkbox"]')[1]);
    await act(async()=>{await vi.advanceTimersByTimeAsync(350);});
    await click(button("Create selected workflows"));
    expect(mocks.applyOnboarding).toHaveBeenCalledWith(["Weekly report"],"Weekly report");
    expect(host.textContent).toContain("Your workflows are ready");
    expect(host.textContent).toContain("Nothing has been run or scheduled");
    await click(button("Open workflows")); expect(done).toHaveBeenCalledWith(true);
  });

  it("preserves an empty selection and disables creation", async () => {
    ready(); await mount();
    for (const box of host.querySelectorAll('input[type="checkbox"]')) await click(box);
    await act(async()=>{await vi.advanceTimersByTimeAsync(350);});
    expect(stored.progress.selected).toEqual([]);
    expect(button("Create selected workflows").disabled).toBe(true);
  });

  it("can cancel and skip generation without applying a proposal", async () => {
    stored.status="generating"; stored.progress.stage="suggestions";
    await mount(); await click(button("Skip personalization"));
    expect(mocks.cancelOnboarding).toHaveBeenCalledTimes(1);
    expect(done).toHaveBeenCalledWith(false);
    expect(mocks.applyOnboarding).not.toHaveBeenCalled();
  });

  it("requires an available signed-in provider but allows skipping", async () => {
    stored.progress.stage="suggestions";
    await mount(true,[{...provider,logged_in:false}]);
    expect(button("Suggest workflows").disabled).toBe(true);
    expect(button("Skip personalization").disabled).toBe(false);
  });

  it("shows a fresh review requirement when the approved preview became stale", async () => {
    ready(); mocks.applyOnboarding.mockImplementation(async()=>({state:structuredClone(stored),created:[],changed:true}));
    await mount(); await click(button("Create selected workflows"));
    expect(host.querySelector('[role="alert"]')?.textContent).toContain("changed");
    expect(done).not.toHaveBeenCalled();
    expect(mocks.previewOnboarding.mock.calls.length).toBeGreaterThan(1);
  });

  it("returns existing users to their saved answers without forcing workspace setup", async () => {
    ready(); stored.status="complete"; stored.progress.stage="ready"; stored.progress.profile={job:"Editor",tasks:"Review drafts"};
    await mount(false);
    expect(host.querySelector<HTMLInputElement>("input")?.value).toBe("Editor");
    expect(mocks.previewOnboarding).not.toHaveBeenCalled();
    await click(button("Continue"));
    expect(host.textContent).toContain("Build your starting team");
    expect(mocks.finishProjectSetup).not.toHaveBeenCalled();
  });

  it("resumes a skipped partial creation with the previous selection", async () => {
    ready(); stored.status="partial"; stored.progress.stage="ready"; stored.progress.skip=true;
    stored.progress.selected=["Short summary"];
    await mount(false);
    expect(host.querySelectorAll("article")).toHaveLength(2);
    expect([...host.querySelectorAll<HTMLInputElement>('input[type="checkbox"]')].map(b => b.checked)).toEqual([false,true]);
    expect(button("Retry remaining creation").disabled).toBe(false);
    await click(button("Retry remaining creation"));
    expect(mocks.applyOnboarding).toHaveBeenCalledWith(["Short summary"],"Short summary");
  });

  it("flushes pending selections before creation so a delayed save cannot undo the ready step", async () => {
    ready(); await mount();
    await click(host.querySelectorAll('input[type="checkbox"]')[1]);
    await click(button("Create selected workflows"));
    await act(async()=>{await vi.advanceTimersByTimeAsync(350);});
    expect(stored.progress.stage).toBe("ready");
    expect(stored.progress.selected).toEqual(["Weekly report"]);
  });
});
