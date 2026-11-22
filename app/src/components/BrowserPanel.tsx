import { useProjectApi } from "../projectContext";
import { useEffect, useRef, useState, type KeyboardEvent, type PointerEvent } from "react";
import { useTranslation } from "react-i18next";
import { openUrl } from "@tauri-apps/plugin-opener";
import { type PageState, type Placement } from "../api";
import { dragWidth, panelTitle, pickButton, placement, samePlacement, type BrowserState } from "../browser";

const viewport = () => ({ width: window.innerWidth, height: window.innerHeight });
const LOAD_TIMEOUT_MS = 30_000;

interface Props {
  state: BrowserState;
  onPage: (page: PageState) => void;
  onClose: () => void;
  onStalled: () => void;
  onResize: (width: number) => void;
  onError: (error: unknown) => void;
  picking: boolean;
  pickAvailable: boolean;
  onPick: (on: boolean) => void;
  onShown: () => void;
}

export default function BrowserPanel({
  state,
  onPage,
  onClose,
  onStalled,
  onResize,
  onError,
  picking,
  pickAvailable,
  onPick,
  onShown,
}: Props) {
  const api = useProjectApi();
  const { t } = useTranslation();
  const placeholder = useRef<HTMLDivElement>(null);
  const aside = useRef<HTMLElement>(null);
  const drag = useRef<{ pointerId: number; startX: number; startWidth: number } | null>(null);
  const [draft, setDraft] = useState<string | null>(null);
  const request = state.request;

  const shown = useRef(true);
  useEffect(() => {
    shown.current = true;
    return () => {
      shown.current = false;
    };
  }, []);

  useEffect(() => {
    const el = placeholder.current;
    if (!request || !el) return;
    api
      .browserOpen(request.url, placement(el.getBoundingClientRect(), viewport()))
      .then((page) => {
        if (!shown.current) return api.browserClose();
        onPage(page);
        onShown();
      })
      .catch((e) => {
        if (!shown.current) return;
        onClose();
        onError(e);
      });
  }, [request, onPage, onClose, onError, onShown]);

  useEffect(() => {
    const el = placeholder.current;
    if (request || !el) return;
    api
      .browserShow(placement(el.getBoundingClientRect(), viewport()))
      .then((page) => {
        if (!page || !shown.current) return;
        onPage(page);
        onShown();
      })
      .catch(onError);
  }, []);

  useEffect(() => {
    if (!state.loading) return;
    const timer = setTimeout(onStalled, LOAD_TIMEOUT_MS);
    return () => clearTimeout(timer);
  }, [state.loading, state.url, onStalled]);

  useEffect(() => {
    const el = placeholder.current;
    if (!el) return;
    let last: Placement | null = null;
    const place = () => {
      const next = placement(el.getBoundingClientRect(), viewport());
      if (last && samePlacement(last, next)) return;
      last = next;
      api.browserPlace(next).catch(onError);
    };
    const observer = new ResizeObserver(place);
    observer.observe(el);
    window.addEventListener("resize", place);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", place);
    };
  }, [onError]);

  function onAddressKey(e: KeyboardEvent<HTMLInputElement>) {
    if (e.key === "Enter" && !e.nativeEvent.isComposing) {
      const address = draft ?? state.url;
      const el = placeholder.current;
      setDraft(null);
      if (!el) return;
      const rect = placement(el.getBoundingClientRect(), viewport());
      api.browserNavigate(address, rect).catch(onError);
    } else if (e.key === "Escape") {
      setDraft(null);
      e.currentTarget.blur();
    }
  }

  function onDividerDown(e: PointerEvent<HTMLDivElement>) {
    if (e.button !== 0 || !aside.current) return;
    e.preventDefault();
    e.currentTarget.setPointerCapture(e.pointerId);
    drag.current = { pointerId: e.pointerId, startX: e.clientX, startWidth: aside.current.getBoundingClientRect().width };
    document.body.classList.add("resizing");
  }
  function onDividerMove(e: PointerEvent<HTMLDivElement>) {
    const d = drag.current;
    if (d && d.pointerId === e.pointerId) onResize(dragWidth(d.startWidth, d.startX, e.clientX));
  }
  function onDividerUp(e: PointerEvent<HTMLDivElement>) {
    if (!drag.current) return;
    e.currentTarget.releasePointerCapture(e.pointerId);
    drag.current = null;
    document.body.classList.remove("resizing");
  }

  const iconButton = (key: string, glyph: string, onClick: () => void, disabled = false) => (
    <button className="icon-button" aria-label={t(key)} title={t(key)} onClick={onClick} disabled={disabled}>
      {glyph}
    </button>
  );

  const pick = pickButton({ picking, url: state.url, pickAvailable });

  return (
    <aside ref={aside} className="browser-panel" aria-busy={state.loading}>
      <div
        className="browser-divider"
        onPointerDown={onDividerDown}
        onPointerMove={onDividerMove}
        onPointerUp={onDividerUp}
        onPointerCancel={onDividerUp}
      />
      <div className="browser-body">
        <header className="browser-header">
          <strong className="browser-title">{panelTitle(state) ?? t("browser.title")}</strong>
          {iconButton("browser.close", "×", onClose)}
        </header>
        <div className="browser-toolbar">
          {iconButton("browser.back", "‹", () => void api.browserBack().catch(onError), !state.url)}
          {iconButton("browser.forward", "›", () => void api.browserForward().catch(onError), !state.url)}
          {iconButton("browser.reload", "↻", () => void api.browserReload().catch(onError), !state.url)}
          <button
            className={picking ? "icon-button active" : "icon-button"}
            aria-pressed={picking}
            aria-label={t(pick.label)}
            title={t(pick.label)}
            onClick={() => onPick(!picking)}
            disabled={pick.disabled}
          >
            ⌖
          </button>
          <input
            type="text"
            aria-label={t("browser.address")}
            placeholder={t("browser.address")}
            value={draft ?? state.url}
            spellCheck={false}
            onChange={(e) => setDraft(e.target.value)}
            onFocus={(e) => e.target.select()}
            onBlur={() => setDraft(null)}
            onKeyDown={onAddressKey}
          />
          {iconButton("browser.openInBrowser", "↗", () => void openUrl(state.url).catch(onError), !state.url)}
          {state.loading && <div className="browser-progress" />}
        </div>
        {state.stalled && <p className="browser-note muted">{t("browser.stalled")}</p>}
        <div ref={placeholder} className="browser-page" />
      </div>
    </aside>
  );
}
