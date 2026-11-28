import { useEffect, useRef, type ButtonHTMLAttributes, type KeyboardEvent, type ReactNode, type RefObject } from "react";
import { useTranslation } from "react-i18next";

const paths = {
  task: "M5 3h10v14H5z M8 7h4 M8 10h4 M8 13h2",
  close: "M5 5l10 10 M15 5L5 15",
  attach: "M7 10l5-5a3 3 0 014 4l-7 7a4 4 0 01-6-6l7-7 M6 11l6-6",
  bot: "M4 7h12v9H4z M10 3v4 M7 11h.01 M13 11h.01 M7 14h6",
  workflow: "M3 3h5v5H3z M12 12h5v5h-5z M8 5h6v7 M5 8v6h7",
  play: "M6 4l10 6-10 6z",
  pause: "M6 4v12 M14 4v12",
  stop: "M5 5h10v10H5z",
  more: "M4 10h.01 M10 10h.01 M16 10h.01",
  dependency: "M3 5h5v5H3z M12 10h5v5h-5z M8 7h7v3",
  plus: "M10 4v12 M4 10h12",
  board: "M3 3h4v14H3z M8 3h4v9H8z M13 3h4v12h-4z",
  schedule: "M16 6a7 7 0 10.5 7 M16 2v4h-4 M10 6v4l3 2",
  archive: "M3 3h14v4H3z M4 7v10h12V7 M8 10h4",
  settings: "M3 5h14 M3 15h14 M7 3v4 M13 13v4 M3 10h14 M12 8v4",
  folder: "M2 5h6l2 2h8v10H2z",
  back: "M12 4l-6 6 6 6",
  forward: "M8 4l6 6-6 6",
  reload: "M16 6a7 7 0 10.5 7 M16 2v4h-4",
  external: "M11 3h6v6 M17 3l-9 9 M8 4H3v13h13v-5",
  target: "M10 1v4 M10 15v4 M1 10h4 M15 10h4 M10 5a5 5 0 100 10 5 5 0 000-10",
  skill: "M10 2l2.5 5.5L18 10l-5.5 2.5L10 18l-2.5-5.5L2 10l5.5-2.5z",
} as const;

export type IconName = keyof typeof paths;
export function Icon({ name }: { name: IconName }) {
  return <svg className="ui-icon" width="18" height="18" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d={paths[name]} /></svg>;
}

export function Button({ variant = "secondary", className = "", ...props }: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: "primary" | "secondary" | "ghost" | "danger" }) {
  return <button type="button" {...props} className={`ui-button ${variant} ${className}`.trim()} />;
}

export function PageHeader({ title, description, actions }: { title: string; description?: string; actions?: ReactNode }) {
  return <header className="page-header"><div className="page-heading"><h1>{title}</h1>{description && <p className="muted">{description}</p>}</div>{actions && <div className="header-actions">{actions}</div>}</header>;
}

export function EditorSection({ title, children }: { title: string; children: ReactNode }) {
  return <section className="editor-section"><h2>{title}</h2><div className="editor-section-content">{children}</div></section>;
}

export function Field({ label, children, hint, error, className = "" }: { label: string; children: ReactNode; hint?: string; error?: ReactNode; className?: string }) {
  return <label className={`form-field ${className}`.trim()}><span className="field-label">{label}</span>{children}{hint && <span className="field-hint muted">{hint}</span>}{error}</label>;
}

export function StatusBadge({ children }: { children: ReactNode }) {
  return <span className="badge">{children}</span>;
}

export function ActionMenu({ label, children }: { label: string; children: ReactNode }) {
  const ref = useRef<HTMLDetailsElement>(null);
  const close = () => { if (ref.current) ref.current.open = false; };
  return <details ref={ref} className="action-menu" onBlur={(event) => {
    if (!event.currentTarget.contains(event.relatedTarget as Node | null)) close();
  }} onKeyDown={(event) => {
    if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); close(); ref.current?.querySelector("summary")?.focus(); }
  }}>
    <summary aria-label={label} title={label}><Icon name="more" /></summary>
    <div className="action-menu-items" onClick={(event) => {
      if ((event.target as HTMLElement).closest("button")) { close(); ref.current?.querySelector("summary")?.focus(); }
    }}>{children}</div>
  </details>;
}

export function PropertyButton({ icon, children, ...props }: ButtonHTMLAttributes<HTMLButtonElement> & { icon: IconName }) {
  return <button type="button" {...props} className="property-control"><Icon name={icon} />{children}</button>;
}

interface DialogProps {
  children: ReactNode;
  labelledBy: string;
  onRequestClose: () => void;
  className?: string;
  initialFocus?: RefObject<HTMLElement>;
  dialogRef?: RefObject<HTMLDialogElement>;
  onKeyDown?: (event: KeyboardEvent<HTMLDialogElement>) => void;
  submitOnShortcut?: boolean;
}

/** Native modal semantics keep background controls inert and contain keyboard focus. */
export function Dialog({ children, labelledBy, onRequestClose, className = "", initialFocus, dialogRef, onKeyDown, submitOnShortcut = false }: DialogProps) {
  const local = useRef<HTMLDialogElement>(null);
  const ref = dialogRef ?? local;
  useEffect(() => {
    const previous = document.activeElement;
    const element = ref.current!;
    element.showModal();
    (initialFocus?.current ?? element.querySelector<HTMLElement>("[data-autofocus]") ?? element.querySelector<HTMLElement>("button:not(:disabled)"))?.focus();
    return () => {
      element.close();
      if (previous instanceof HTMLElement && previous.isConnected) previous.focus();
    };
  }, [ref, initialFocus]);
  function keys(event: KeyboardEvent<HTMLDialogElement>) {
    onKeyDown?.(event);
    if (!event.defaultPrevented && submitOnShortcut && event.key === "Enter" && (event.metaKey || event.ctrlKey) && !event.nativeEvent.isComposing) {
      event.preventDefault();
      ref.current?.querySelector("form")?.requestSubmit();
    }
  }
  return <dialog ref={ref} className={`ui-dialog ${className}`.trim()} aria-labelledby={labelledBy} onKeyDown={keys}
    onCancel={(event) => { event.preventDefault(); onRequestClose(); }}>{children}</dialog>;
}

export function DialogHeader({ id, title, icon, onClose, locked = false }: { id: string; title: string; icon: IconName; onClose: () => void; locked?: boolean }) {
  const { t } = useTranslation();
  return <header className="dialog-header"><div className="dialog-heading"><Icon name={icon} /><h2 id={id}>{title}</h2></div>
    <Button variant="ghost" className="icon-button" disabled={locked} aria-label={t("board.close")} title={t("board.close")} onClick={onClose}><Icon name="close" /></Button>
  </header>;
}
