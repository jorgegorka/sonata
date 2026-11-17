(() => {
  "use strict";
  if (globalThis.__sonataPick) return;

  const TITLE_MAX = 300;
  const TEXT_MAX = 300;
  const HTML_MAX = 1500;
  const ATTRIBUTE_MAX = 120;
  const URL_MAX = 8192;
  const NAME_MAX = 100;
  const DEEP_ELEMENTS_MAX = 1000;
  const DEEP_HTML_MAX = 20000;
  const SELECTOR_DEPTH = 12;
  const EDGE_MAX = 500000;
  const REATTACH_MAX = 3;

  const SECRET_ATTRIBUTE = /token|csrf|nonce|secret|key|password|auth|sig/i;
  const SECRET_PARAMETER = /token|csrf|nonce|secret|key|password|auth|code|sig/i;
  const STABLE_ATTRIBUTES = ["data-testid", "data-test", "data-qa", "data-cy", "name", "aria-label"];
  const TAG = /^[a-z][a-z0-9-]{0,63}$/;
  const HIDDEN_TEXT = new Set(["script", "style", "noscript", "template"]);
  const INLINE = new Set(["a", "abbr", "b", "bdi", "bdo", "cite", "code", "data", "dfn", "em", "font", "i", "kbd",
    "label", "mark", "q", "s", "samp", "small", "span", "strong", "sub", "sup", "time", "u", "var", "wbr"]);
  const FRAMES = ["iframe", "frame", "embed", "object"];
  const SWALLOWED = ["pointerdown", "pointerup", "mousedown", "mouseup", "click", "dblclick", "auxclick",
    "contextmenu", "touchstart", "touchend"];

  function cut(text, max) {
    const points = [];
    for (const ch of text) {
      if (points.length === max) {
        points.length = max - 1;
        points.push("…");
        break;
      }
      points.push(ch.length === 1 && ch >= "\uD800" && ch <= "\uDFFF" ? "\uFFFD" : ch);
    }
    return points.join("");
  }

  const wellFormed = (text) =>
    text.replace(/[\uD800-\uDBFF][\uDC00-\uDFFF]|[\uD800-\uDFFF]/g, (ch) => (ch.length === 2 ? ch : "\uFFFD"));

  function longerThan(text, max) {
    if (text.length <= max) return false;
    let count = 0;
    for (const _ of text) if (++count > max) return true;
    return false;
  }

  function cssSelector(el) {
    const doc = el.ownerDocument;
    try {
      if (hasUniqueId(el)) return `#${CSS.escape(el.id)}`;
      let sel = "";
      for (let node = el, depth = 0; node && depth < SELECTOR_DEPTH; node = node.parentElement, depth++) {
        const anchor = node !== el && hasUniqueId(node);
        sel = anchor ? `#${CSS.escape(node.id)} > ${sel}` : sel ? `${segment(node)} > ${sel}` : segment(node);
        if (matchesOnly(doc, sel, el)) return sel;
        if (anchor) break;
      }
    } catch {
    }
    return fullPath(el);
  }

  const usable = (name) => Boolean(name) && wellFormed(name) === name && !longerThan(name, NAME_MAX);

  function hasUniqueId(el) {
    return usable(el.id) && el.ownerDocument.querySelectorAll(`#${CSS.escape(el.id)}`).length === 1;
  }

  function matchesOnly(doc, sel, el) {
    const found = doc.querySelectorAll(sel);
    return found.length === 1 && found[0] === el;
  }

  function segment(node) {
    let sel = CSS.escape(node.localName);
    const attribute = STABLE_ATTRIBUTES.find((name) => usable(node.getAttribute(name)));
    if (attribute) {
      sel += `[${attribute}="${quoteCss(node.getAttribute(attribute))}"]`;
    } else {
      const classes = Array.from(node.classList).filter((name) => usable(name) && !looksGenerated(name));
      for (const name of classes.slice(0, 2)) sel += `.${CSS.escape(name)}`;
    }
    const parent = node.parentElement;
    if (!parent) return sel;
    let index = 0;
    let seen = 0;
    let twin = false;
    for (const sibling of parent.children) {
      if (sibling.localName !== node.localName) continue;
      seen += 1;
      if (sibling === node) index = seen;
      else if (!twin && sibling.matches(sel)) twin = true;
    }
    return twin ? `${sel}:nth-of-type(${index})` : sel;
  }

  function fullPath(el) {
    const steps = [];
    for (let node = el; node; node = node.parentElement) {
      const tag = CSS.escape(node.localName);
      const parent = node.parentElement;
      const same = parent ? Array.from(parent.children).filter((s) => s.localName === node.localName) : [node];
      steps.unshift(same.length > 1 ? `${tag}:nth-of-type(${same.indexOf(node) + 1})` : tag);
    }
    return steps.join(" > ");
  }

  const looksGenerated = (name) => /\d{3}|^(css|sc|jsx)-|^_|__(?=[a-z0-9]*\d)[a-z0-9]{5}/i.test(name);

  const quoteCss = (value) =>
    value.replace(/["\\]/g, "\\$&").replace(/[\n\r\f]/g, (ch) => `\\${ch.charCodeAt(0).toString(16)} `);

  function redactHtml(el, max = HTML_MAX) {
    const deep = isSmall(el) && el.outerHTML.length <= DEEP_HTML_MAX;
    const copy = el.ownerDocument.implementation.createHTMLDocument("").importNode(el, deep);
    if (deep) for (const node of copy.querySelectorAll("[data-sonata-pick]")) node.remove();
    if (!deep) copy.append("…");
    for (const node of [copy, ...copy.querySelectorAll("*")]) redactAttributes(node);
    return cut(copy.outerHTML, max);
  }

  function isSmall(root, budget = { elements: DEEP_ELEMENTS_MAX, chars: DEEP_HTML_MAX }) {
    const show = NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT | NodeFilter.SHOW_COMMENT;
    const walker = root.ownerDocument.createTreeWalker(root, show);
    for (let node = root; node; node = walker.nextNode()) {
      if (node.nodeType === Node.ELEMENT_NODE) {
        if (node !== root) budget.elements -= 1;
        for (const attribute of Array.from(node.attributes)) {
          budget.chars -= attribute.name.length + attribute.value.length;
        }
        if (node.localName === "template" && node.content && !isSmall(node.content, budget)) return false;
      } else if (node.nodeType !== Node.DOCUMENT_FRAGMENT_NODE) {
        budget.chars -= node.data.length;
      }
      if (budget.elements < 0 || budget.chars < 0) return false;
    }
    return true;
  }

  function redactAttributes(node) {
    const type = (node.getAttribute("type") || "").toLowerCase();
    const secretField =
      (node.localName === "input" && (type === "hidden" || type === "password")) ||
      SECRET_ATTRIBUTE.test(node.getAttribute("name") || "");
    for (const attribute of Array.from(node.attributes)) {
      const named = attribute.name === "value" || attribute.name === "content";
      if (SECRET_ATTRIBUTE.test(attribute.name) || (secretField && named)) attribute.value = "redacted";
      else if (attribute.value.length > ATTRIBUTE_MAX) attribute.value = cut(attribute.value, ATTRIBUTE_MAX);
    }
  }

  function elementText(el, max) {
    const walker = el.ownerDocument.createTreeWalker(el, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT, {
      acceptNode: (node) => (HIDDEN_TEXT.has(node.localName) ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT),
    });
    const separates = (node) => node?.nodeType === Node.ELEMENT_NODE && !INLINE.has(node.localName);
    let text = "";
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      if (node.nodeType === Node.ELEMENT_NODE) {
        if (separates(node)) text += " ";
        continue;
      }
      if (separates(node.previousSibling)) text += " ";
      for (let i = 0; i < node.data.length; i += 4096) {
        text = (text + node.data.slice(i, i + 4096)).replace(/\s+/g, " ");
        if (longerThan(text.trimStart(), max)) return cut(text.trim(), max);
      }
    }
    return cut(text.trim(), max);
  }

  function redactUrl(href) {
    let url;
    try {
      url = new URL(href);
    } catch {
      return href;
    }
    const search = redactParameters(url.search.slice(1));
    const hash = url.hash.includes("=") ? redactParameters(url.hash.slice(1)) : null;
    if (search !== null) url.search = search;
    if (hash !== null) url.hash = hash;
    const redacted = search === null && hash === null ? href : url.href;
    if (redacted.length <= URL_MAX) return redacted;
    url.search = "";
    url.hash = "";
    return url.href;
  }

  function redactParameters(query) {
    let changed = false;
    const pairs = query.split("&").map((pair) => {
      const eq = pair.indexOf("=");
      if (eq < 1 || eq === pair.length - 1 || !SECRET_PARAMETER.test(decodeName(pair.slice(0, eq)))) return pair;
      changed = true;
      return `${pair.slice(0, eq)}=redacted`;
    });
    return changed ? pairs.join("&") : null;
  }

  function decodeName(name) {
    try {
      return decodeURIComponent(name.replace(/\+/g, " "));
    } catch {
      return name;
    }
  }

  function keyAction(e, mode) {
    if (!e.isTrusted) return "ignore";
    if (e.metaKey || e.ctrlKey || e.altKey || e.isComposing || e.keyCode === 229) return "pass";
    if (mode === "picked") return e.key === "Escape" ? "close" : "pass";
    if (e.key === "ArrowUp") return "parent";
    if (e.key === "ArrowDown") return "child";
    return e.key === "Escape" ? "end" : "pass";
  }

  const STYLES = `
    div { box-sizing: border-box; }
    [hidden] { display: none !important; }
    .shield { position: fixed; left: 0; top: 0; pointer-events: auto; user-select: none; -webkit-user-select: none; }
    .highlight, .picked, .label, .hint { position: fixed; pointer-events: none; }
    .highlight { left: 0; top: 0; border: 2px solid #2563eb; border-radius: 2px; background: rgba(37, 99, 235, 0.12); }
    .picked { left: 0; top: 0; border: 2px solid #2563eb; border-radius: 2px;
      box-shadow: 0 0 0 2px rgba(255, 255, 255, 0.9), 0 0 0 4px rgba(37, 99, 235, 0.45); }
    .label { left: 0; top: 0; padding: 1px 6px; border-radius: 3px; background: #1e3a8a; color: #fff;
      font: 11px/16px ui-monospace, Menlo, monospace; white-space: nowrap; }
    .hint { left: 50%; bottom: 12px; transform: translateX(-50%); max-width: calc(100% - 24px); padding: 6px 12px;
      border-radius: 6px; background: rgba(17, 24, 39, 0.92); color: #fff;
      font: 12px/1.4 -apple-system, BlinkMacSystemFont, sans-serif; white-space: nowrap; overflow: hidden;
      text-overflow: ellipsis; }`;

  const HOST_STYLE = {
    all: "initial",
    display: "block",
    position: "fixed",
    top: "0",
    right: "0",
    bottom: "0",
    left: "0",
    width: "auto",
    height: "auto",
    margin: "0",
    padding: "0",
    border: "0",
    background: "transparent",
    overflow: "visible",
    direction: "ltr",
    "z-index": "2147483647",
    "pointer-events": "none",
  };

  let on = false;
  let hint = "";
  let host = null;
  let ui = null;
  let observer = null;
  let reattached = [];
  let frames = [];
  let dialogs = null;
  let openings = 0;
  let opened = new WeakMap();
  let listening = [];
  let frame = 0;
  let full = false;
  let pointer = null;
  let hovered = null;
  let current = null;
  let path = [];
  let picked = null;
  let pickedRect = null;

  function start(options) {
    try {
      hint = options.strings.hint;
      if (on) return refresh();
      on = true;
      if (document.readyState === "loading") listen(document, "DOMContentLoaded", mount, { once: true });
      else mount();
    } catch {
      end();
    }
  }

  function mount() {
    if (!on || host) return;
    try {
      buildOverlay();
      frames = FRAMES.map((tag) => document.getElementsByTagName(tag));
      dialogs = document.getElementsByTagName("dialog");
      attachHost(hostParent());
      observer = new MutationObserver(safely(onMutation));
      observer.observe(document.documentElement, { childList: true, subtree: true, attributeFilter: ["open"] });
      listen(window, "pointermove", onPointerMove, { capture: true, passive: true });
      for (const type of SWALLOWED) listen(window, type, onSwallowed, { capture: true, passive: false });
      listen(window, "keydown", onKeyDown, { capture: true });
      listen(window, "scroll", schedule, { capture: true, passive: true });
      listen(window, "resize", schedule, { passive: true });
      listen(window, "close", placeHost, { capture: true });
      listen(window, "pagehide", stop);
      refresh();
    } catch {
      end();
    }
  }

  function listen(emitter, type, listener, options) {
    const safe = safely(listener);
    emitter.addEventListener(type, safe, options);
    listening.push([emitter, type, safe, options]);
  }

  function safely(fn) {
    return (...args) => {
      try {
        return fn(...args);
      } catch {
        return undefined;
      }
    };
  }

  function stop() {
    on = false;
    for (const [emitter, type, listener, options] of listening) emitter.removeEventListener(type, listener, options);
    listening = [];
    observer?.disconnect();
    observer = null;
    if (frame) cancelAnimationFrame(frame);
    frame = 0;
    full = false;
    host?.remove();
    host = ui = pointer = hovered = current = picked = pickedRect = null;
    path = [];
    frames = [];
    dialogs = null;
    opened = new WeakMap();
    reattached = [];
  }

  function end() {
    stop();
    post({ type: "ended" });
  }

  function post(message) {
    const json = JSON.stringify(message, (_, value) => (typeof value === "string" ? wellFormed(value) : value));
    window.webkit?.messageHandlers?.sonataPick?.postMessage(json);
  }

  function onMutation(records) {
    for (const record of records) {
      if (record.type === "attributes" && record.target.localName === "dialog" && record.target.open) {
        opened.set(record.target, ++openings);
      }
    }
    placeHost();
    schedule();
  }

  function placeHost() {
    if (!host) return;
    const parent = hostParent();
    if (host.isConnected && host.parentNode === parent) return;
    if (host.parentNode === null) {
      const now = performance.now();
      reattached = reattached.filter((time) => now - time < 1000);
      if (reattached.length >= REATTACH_MAX) return end();
      reattached.push(now);
    }
    attachHost(parent);
  }

  function hostParent() {
    let top = null;
    for (const dialog of dialogs) {
      if (!isModal(dialog)) continue;
      if (top === null || (opened.get(dialog) ?? 0) >= (opened.get(top) ?? 0)) top = dialog;
    }
    return top ?? document.documentElement;
  }

  function isModal(dialog) {
    try {
      return dialog.matches(":modal");
    } catch {
      return false;
    }
  }

  function make(tag, className, parent) {
    const el = document.createElement(tag);
    el.className = className;
    parent.append(el);
    return el;
  }

  function buildOverlay() {
    host = document.createElement("div");
    host.dataset.sonataPick = "";
    for (const [name, value] of Object.entries(HOST_STYLE)) host.style.setProperty(name, value, "important");
    if (typeof host.showPopover === "function") host.popover = "manual";
    const shadow = host.attachShadow({ mode: "closed" });
    adoptStyles(shadow);
    ui = { shieldList: [] };
    ui.shields = make("div", "shields", shadow);
    ui.highlight = make("div", "highlight", shadow);
    ui.picked = make("div", "picked", shadow);
    ui.label = make("div", "label", shadow);
    ui.hint = make("div", "hint", shadow);
    ui.highlight.hidden = ui.picked.hidden = ui.label.hidden = true;
    for (const type of ["pointerdown", "mousedown", "click", "dblclick", "auxclick", "contextmenu"]) {
      ui.shields.addEventListener(type, safely(onShield));
    }
  }

  function adoptStyles(shadow) {
    if ("adoptedStyleSheets" in shadow && typeof CSSStyleSheet.prototype.replaceSync === "function") {
      const sheet = new CSSStyleSheet();
      sheet.replaceSync(STYLES);
      shadow.adoptedStyleSheets = [sheet];
    } else {
      const style = document.createElement("style");
      style.textContent = STYLES;
      shadow.append(style);
    }
  }

  function attachHost(parent) {
    parent.append(host);
    if (host.popover) {
      try {
        host.hidePopover();
      } catch {
      }
      host.showPopover();
    }
  }

  const fromOverlay = (e) => host !== null && e.composedPath().includes(host);

  function refresh() {
    if (!ui) return;
    ui.hint.textContent = hint;
    schedule();
  }

  function onPointerMove(e) {
    if (!e.isTrusted) return;
    pointer = { x: e.clientX, y: e.clientY };
    if (!picked) schedule();
  }

  function schedule() {
    full = true;
    request();
  }

  function request() {
    if (ui && !frame) frame = requestAnimationFrame(safely(onFrame));
  }

  function onFrame() {
    frame = 0;
    if (full) render();
    else if (ui && picked) follow();
  }

  function render() {
    if (frame) cancelAnimationFrame(frame);
    frame = 0;
    full = false;
    if (!ui) return;
    placeShields();
    if (picked) {
      ui.highlight.hidden = ui.label.hidden = ui.hint.hidden = true;
      follow();
      return;
    }
    ui.picked.hidden = true;
    ui.hint.hidden = false;
    if (pointer) hover(hitTest(pointer.x, pointer.y));
    if (current?.isConnected) draw(current);
    else ui.highlight.hidden = ui.label.hidden = true;
  }

  function placeShields() {
    let used = 0;
    for (const list of frames) {
      for (const el of list) {
        const r = el.getBoundingClientRect();
        if (r.width <= 0 || r.height <= 0 || r.bottom <= 0 || r.right <= 0) continue;
        if (r.top >= innerHeight || r.left >= innerWidth) continue;
        if (used === ui.shieldList.length) ui.shieldList.push(make("div", "shield", ui.shields));
        const shield = ui.shieldList[used++];
        shield.hidden = false;
        setBox(shield, { left: r.left, top: r.top, width: r.width, height: r.height });
      }
    }
    for (const shield of ui.shieldList.slice(used)) shield.hidden = true;
  }

  function hitTest(x, y) {
    const stack = document.elementsFromPoint ? document.elementsFromPoint(x, y) : [document.elementFromPoint(x, y)];
    return stack.find((el) => el && el !== host) ?? null;
  }

  function hover(el) {
    if (el === hovered) return;
    hovered = current = el;
    path = [];
  }

  function draw(el) {
    const r = el.getBoundingClientRect();
    setBox(ui.highlight, { left: r.left, top: r.top, width: r.width, height: r.height });
    ui.highlight.hidden = ui.label.hidden = false;
    ui.label.textContent = describe(el, r);
    const width = ui.label.offsetWidth;
    const height = ui.label.offsetHeight;
    setBox(ui.label, {
      left: Math.max(0, Math.min(r.left, innerWidth - width)),
      top: r.top >= height + 2 ? r.top - height - 2 : Math.max(r.top, 0) + 2,
    });
  }

  function describe(el, r) {
    const classes = Array.from(el.classList)
      .slice(0, 2)
      .map((name) => `.${name}`)
      .join("");
    const name = cut(`${el.localName}${el.id ? `#${el.id}` : ""}${classes}`, 80);
    return `${name} ${Math.round(r.width)}×${Math.round(r.height)}`;
  }

  function setBox(el, box) {
    for (const [name, value] of Object.entries(box)) el.style.setProperty(name, `${value}px`);
  }

  function widen() {
    if (!current?.parentElement) return;
    path.push(current);
    current = current.parentElement;
    schedule();
  }

  function narrow() {
    if (path.length === 0) return;
    current = path.pop();
    schedule();
  }

  function onSwallowed(e) {
    if (!e.isTrusted || fromOverlay(e)) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    if (e.type === "click" && e.detail > 0) pickAt(e.clientX, e.clientY);
  }

  function onShield(e) {
    if (!e.isTrusted) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.type === "click" && e.detail > 0) pickAt(e.clientX, e.clientY);
  }

  function onKeyDown(e) {
    const action = keyAction(e, picked ? "picked" : "hover");
    if (action === "ignore" || action === "pass") return;
    e.preventDefault();
    e.stopImmediatePropagation();
    if (action === "parent") widen();
    else if (action === "child") narrow();
    else if (action === "end") end();
    else if (action === "close") {
      closed();
      post({ type: "close" });
    }
  }

  function pickAt(x, y) {
    if (picked) {
      hovered = current = hitTest(x, y);
      path = [];
    } else if (path.length === 0) {
      hover(hitTest(x, y));
    }
    if (current) pickElement(current);
  }

  function pickElement(el) {
    const element = details(el);
    picked = el;
    pickedRect = rectOf(el);
    post({ type: "picked", element, rect: pickedRect });
    render();
  }

  function follow() {
    outline();
    request();
  }

  function outline() {
    if (!picked.isConnected) {
      ui.picked.hidden = true;
      return;
    }
    const rect = rectOf(picked);
    setBox(ui.picked, { left: rect.x, top: rect.y, width: rect.width, height: rect.height });
    ui.picked.hidden = false;
    if (sameRect(rect, pickedRect)) return;
    pickedRect = rect;
    post({ type: "moved", rect });
  }

  function rectOf(el) {
    const r = el.getBoundingClientRect();
    const edge = (value) => Math.max(-EDGE_MAX, Math.min(value, EDGE_MAX));
    const left = edge(r.left);
    const top = edge(r.top);
    return { x: left, y: top, width: edge(r.right) - left, height: edge(r.bottom) - top };
  }

  const sameRect = (a, b) => a.x === b.x && a.y === b.y && a.width === b.width && a.height === b.height;

  function details(el) {
    const tag = el.localName.toLowerCase();
    return {
      url: redactUrl(location.href),
      title: cut(document.title, TITLE_MAX),
      selector: cssSelector(el),
      tag: TAG.test(tag) ? tag : "element",
      text: elementText(el, TEXT_MAX),
      html: redactHtml(el),
    };
  }

  function closed() {
    picked = pickedRect = null;
    render();
  }

  globalThis.__sonataPick = {
    start,
    stop: safely(stop),
    closed: safely(closed),
    sent: safely(closed),
    _internals: { cut, cssSelector, redactHtml, redactUrl, keyAction },
  };
})();
