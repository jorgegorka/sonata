import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { PickBoxState } from "../api";
import { boxKey, boxView } from "./box";
import "./pickbox.css";

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const target = $("target");
const comment = $<HTMLTextAreaElement>("comment");
const keys = $("keys");
const status = $("status");
const send = $<HTMLButtonElement>("send");

let state: PickBoxState | null = null;
let generation: number | null = null;
let refused = false;

function render() {
  if (!state) return;
  const strings = state.strings;
  const shown: PickBoxState = refused && state.status === "idle" ? { ...state, status: "failed" } : state;
  const view = boxView(shown, comment.value);
  target.textContent = view.target;
  target.title = view.target;
  comment.placeholder = strings?.placeholder ?? "";
  keys.textContent = strings?.keys ?? "";
  send.textContent = strings?.send ?? "";
  send.disabled = !view.canSend;
  status.textContent = view.status ?? "";
  status.classList.toggle("failed", view.alert);
}

function apply(next: PickBoxState) {
  if (generation !== null && next.generation !== generation) comment.value = "";
  generation = next.generation;
  state = next;
  refused = false;
  render();
}

const quiet = () => {};
const submit = () => {
  if (!state || !boxView(state, comment.value).canSend) return;
  refused = false;
  render();
  void invoke("browser_pick_submit", { comment: comment.value }).catch(() => {
    refused = true;
    render();
  });
};
const close = () => void invoke("browser_pick_close").catch(quiet);

document.addEventListener("keydown", (e) => {
  const action = boxKey(e);
  if (action === "close") {
    e.preventDefault();
    close();
  } else if (action === "send" && e.target === comment) {
    e.preventDefault();
    submit();
  }
});
comment.addEventListener("input", render);
send.addEventListener("click", submit);
const focusComment = () => comment.focus();
window.addEventListener("focus", focusComment);
const keepTyping = (e: Event) => {
  if (e.target === send || e.target === comment) return;
  e.preventDefault();
  focusComment();
};
document.addEventListener("pointerdown", keepTyping);
document.addEventListener("mousedown", keepTyping);

listen<PickBoxState>("pickbox-state", (e) => apply(e.payload))
  .then(() => invoke<PickBoxState>("browser_pick_box_state"))
  .then((initial) => {
    if (state === null) apply(initial);
  })
  .catch(quiet);
focusComment();
