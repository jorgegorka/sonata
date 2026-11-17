// @vitest-environment happy-dom
/// <reference types="vite/client" />
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import source from "../src-tauri/src/browser_pick.js?raw";

new Function(source)();
const h = (globalThis as any).__sonataPick._internals;

const lone = /[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/;

describe("the page picker", () => {
  it("cuts by code points and never leaves half an emoji", () => {
    expect(h.cut("a😀b", 2)).toBe("a…");
    expect(h.cut("abc", 3)).toBe("abc");
    for (let n = 1; n < 12; n++) expect(lone.test(h.cut("😀é😀x😀".repeat(3), n))).toBe(false);
  });
  it("replaces a lone surrogate from the page, which the JSON for Rust could not carry", () => {
    expect(h.cut("a\uD800b", 10)).toBe("a�b");
    expect(lone.test(h.cut("\uDC00".repeat(20), 5))).toBe(false);
  });
  it("names an element by a unique id, a test attribute, or a verified path", () => {
    document.body.innerHTML = `<button id="save">S</button><button data-testid="go">G</button>
      <div class="css-1x2y3z card"><span>x</span></div><div class="other"><span>y</span></div>
      <table><tr><td>1</td><td>2</td><td>3</td></tr><tr><td>4</td><td>5</td><td>6</td></tr><tr><td>7</td><td>8</td><td>9</td></tr></table>
      <p id="a:b">weird</p>`;
    expect(h.cssSelector(document.getElementById("save"))).toBe("#save");
    expect(h.cssSelector(document.querySelector("[data-testid=go]"))).toBe('button[data-testid="go"]');
    expect(h.cssSelector(document.getElementById("a:b"))).toBe("#a\\:b");
    const cell = document.querySelectorAll("td")[4];
    const sel = h.cssSelector(cell);
    expect(document.querySelectorAll(sel)).toHaveLength(1);
    expect(document.querySelector(sel)).toBe(cell);
    expect(h.cssSelector(document.querySelector(".card span"))).toBe("div.card > span");
  });
  it("keeps BEM classes and leaves out hashed ones", () => {
    document.body.innerHTML = `<i class="card__title">a</i><i>b</i><b class="Button_main__a1B2c">c</b><b>d</b>`;
    expect(h.cssSelector(document.querySelector(".card__title"))).toBe("i.card__title");
    expect(h.cssSelector(document.querySelector(".Button_main__a1B2c"))).toBe("b:nth-of-type(1)");
  });
  it("leaves long or broken names out of a selector", () => {
    document.body.innerHTML = `<div><a aria-label="${"x".repeat(300)}">1</a><a>2</a></div><p>x</p><p>y</p>`;
    const link = document.querySelector("a")!;
    const linkSel = h.cssSelector(link);
    expect(linkSel).not.toContain("aria-label");
    expect(document.querySelector(linkSel)).toBe(link);
    const p = document.querySelectorAll("p")[1];
    p.id = "a\uD800";
    p.setAttribute("data-testid", "b\uDC00");
    p.className = "c\uD800";
    const sel = h.cssSelector(p);
    expect(lone.test(sel)).toBe(false);
    expect(document.querySelector(sel)).toBe(p);
    expect(lone.test(h.redactHtml(p))).toBe(false);
  });
  it("blanks secrets and long attributes in the HTML", () => {
    document.body.innerHTML = `<form><input type="hidden" name="authenticity_token" value="abc">
      <input type="password" value="hunter2"><div data-api-key="k1" title="${"t".repeat(200)}">x</div></form>`;
    const html = h.redactHtml(document.querySelector("form"));
    expect(html).not.toMatch(/abc|hunter2|k1/);
    expect(html).toContain('data-api-key="redacted"');
    expect(html).toContain(`title="${"t".repeat(119)}…"`);
  });
  it("blanks the value of a field or meta tag named like a secret", () => {
    document.head.innerHTML =
      `<meta name="csrf-token" content="s3cr3t"><meta name="viewport" content="width=device-width">`;
    document.body.innerHTML = `<input name="api_key" value="k2">`;
    const html = h.redactHtml(document.documentElement);
    expect(html).not.toMatch(/s3cr3t|k2/);
    expect(html).toContain('content="width=device-width"');
    document.head.innerHTML = "";
  });
  it("keeps a huge element short and quick", () => {
    document.body.innerHTML = `<main>${"<p>lorem ipsum</p>".repeat(8000)}</main>`;
    const started = performance.now();
    const html = h.redactHtml(document.body);
    expect(performance.now() - started).toBeLessThan(200);
    expect([...html].length).toBeLessThanOrEqual(1500);
    expect(html.startsWith("<body")).toBe(true);
  });
  it("blanks secret-looking URL parameters", () => {
    expect(h.redactUrl("https://a.test/cb?code=123&state=x#access_token=abc&x=1")).toBe(
      "https://a.test/cb?code=redacted&state=x#access_token=redacted&x=1",
    );
    expect(h.redactUrl("https://a.test/docs#section-2")).toBe("https://a.test/docs#section-2");
    expect(h.redactUrl("http://localhost:3000/orders?page=2")).toBe("http://localhost:3000/orders?page=2");
  });
  it("drops the query and the fragment of a URL too long for the app", () => {
    expect(h.redactUrl(`https://a.test/p?x=${"y".repeat(9000)}#z`)).toBe("https://a.test/p");
  });
  it("keeps the other parameters of a redacted URL as they were", () => {
    expect(h.redactUrl("https://a.test/s?q=a%20b+c&token=t&tags=x,y")).toBe(
      "https://a.test/s?q=a%20b+c&token=redacted&tags=x,y",
    );
  });
  it("maps keys to actions", () => {
    const k = (key: string, more = {}) => ({ key, shiftKey: false, metaKey: false, ctrlKey: false, altKey: false,
      isComposing: false, keyCode: 0, isTrusted: true, ...more });
    expect(h.keyAction(k("ArrowUp"), "hover")).toBe("parent");
    expect(h.keyAction(k("ArrowDown"), "hover")).toBe("child");
    expect(h.keyAction(k("Escape"), "hover")).toBe("end");
    expect(h.keyAction(k("c", { metaKey: true, shiftKey: true }), "hover")).toBe("pass");
    expect(h.keyAction(k("Escape", { isTrusted: false }), "hover")).toBe("ignore");
    expect(h.keyAction(k("Escape"), "picked")).toBe("close");
    expect(h.keyAction(k("Escape", { isTrusted: false }), "picked")).toBe("ignore");
    expect(h.keyAction(k("ArrowUp"), "picked")).toBe("pass");
    expect(h.keyAction(k("Enter"), "picked")).toBe("pass");
  });
  it("leaves the keys of an input method that is composing to it", () => {
    const k = (key: string, more = {}) => ({ key, shiftKey: false, metaKey: false, ctrlKey: false, altKey: false,
      isComposing: false, keyCode: 0, isTrusted: true, ...more });
    for (const mode of ["hover", "picked"]) {
      expect(h.keyAction(k("Escape", { isComposing: true }), mode)).toBe("pass");
      expect(h.keyAction(k("Escape", { keyCode: 229 }), mode)).toBe("pass");
    }
    expect(h.keyAction(k("ArrowUp", { isComposing: true }), "hover")).toBe("pass");
    expect(h.keyAction(k("ArrowDown", { keyCode: 229 }), "hover")).toBe("pass");
  });
  it("starts once and stops cleanly", () => {
    const pick = (globalThis as any).__sonataPick;
    pick.start({ strings: { hint: "h" }, target: "To @a · b" });
    pick.start({ strings: { hint: "h" }, target: "To @a · c" });
    expect(document.querySelectorAll("[data-sonata-pick]")).toHaveLength(1);
    pick.stop();
    expect(document.querySelectorAll("[data-sonata-pick]")).toHaveLength(0);
  });
  it("leaves its own overlay out of a picked <html>", () => {
    const pick = (globalThis as any).__sonataPick;
    document.body.innerHTML = "<p>small page</p>";
    pick.start({ strings: { hint: "h" }, target: null });
    expect(document.querySelectorAll("[data-sonata-pick]")).toHaveLength(1);
    const html = h.redactHtml(document.documentElement);
    expect(html).toContain("small page");
    expect(html).not.toContain("sonata-pick");
    pick.stop();
  });
  it("evaluates again as a no-op, as Rust does before every call", () => {
    const pick = (globalThis as any).__sonataPick;
    new Function(source)();
    expect((globalThis as any).__sonataPick).toBe(pick);
  });
});

describe("the picker's wiring", () => {
  const pick = (globalThis as any).__sonataPick;
  const strings = { hint: "h" };
  const attachShadow = Element.prototype.attachShadow;
  let shadow: ShadowRoot;
  let raw: string[];
  const posts = () => raw.map((m) => JSON.parse(m));
  const types = () => posts().map((m) => m.type);

  const trusted = <T extends Event>(e: T): T => Object.defineProperty(e, "isTrusted", { value: true });
  const frame = () => new Promise((resolve) => setTimeout(resolve, 40));
  const q = <T extends Element = HTMLElement>(selector: string) => shadow.querySelector(selector) as unknown as T;
  const overlay = () => document.querySelector("[data-sonata-pick]");
  const under = (...stack: (Element | null)[]) => {
    (document as any).elementsFromPoint = () => stack;
  };
  const click = (el: Element, init: MouseEventInit = {}) => {
    const e = trusted(new MouseEvent("click", { bubbles: true, cancelable: true, composed: true, detail: 1, ...init }));
    el.dispatchEvent(e);
    return e;
  };
  const key = (el: EventTarget, name: string, init: KeyboardEventInit = {}) => {
    const options = { key: name, bubbles: true, cancelable: true, composed: true, ...init };
    const e = trusted(new KeyboardEvent("keydown", options));
    el.dispatchEvent(e);
    return e;
  };
  const rect = (left: number, top: number, width: number, height: number) => () =>
    ({ left, top, width, height, right: left + width, bottom: top + height, x: left, y: top }) as DOMRect;
  const placed = (el: HTMLElement) => [el.style.left, el.style.top, el.style.width, el.style.height];
  const modal = (dialog: HTMLDialogElement) => {
    const matches = dialog.matches.bind(dialog);
    dialog.matches = (selector: string) => (selector === ":modal" ? dialog.open : matches(selector));
    return dialog;
  };

  beforeEach(() => {
    raw = [];
    (window as any).webkit = { messageHandlers: { sonataPick: { postMessage: (m: string) => raw.push(m) } } };
    Element.prototype.attachShadow = function (this: Element, init: ShadowRootInit) {
      shadow = attachShadow.call(this, init);
      return shadow;
    };
  });
  afterEach(() => {
    pick.stop();
    Element.prototype.attachShadow = attachShadow;
    delete (window as any).webkit;
    delete (document as any).elementsFromPoint;
  });

  it("picks on a trusted click, swallows it, and posts the element and where it is", () => {
    document.body.innerHTML = `<button class="btn">Save order</button>`;
    const button = document.querySelector("button")!;
    button.getBoundingClientRect = rect(100, 120, 80, 30);
    let pageSaw = false;
    button.addEventListener("click", () => (pageSaw = true));
    pick.start({ strings, target: "To @a · b" });
    expect(q(".hint").textContent).toBe("h");
    under(overlay(), button, document.body, document.documentElement);
    const e = click(button);
    expect(pageSaw).toBe(false);
    expect(e.defaultPrevented).toBe(true);
    expect(posts()).toEqual([
      {
        type: "picked",
        element: {
          url: location.href,
          title: document.title,
          selector: "button.btn",
          tag: "button",
          text: "Save order",
          html: '<button class="btn">Save order</button>',
        },
        rect: { x: 100, y: 120, width: 80, height: 30 },
      },
    ]);
    expect(q(".picked").hidden).toBe(false);
    expect(placed(q(".picked"))).toEqual(["100px", "120px", "80px", "30px"]);
    expect([q(".highlight").hidden, q(".label").hidden, q(".hint").hidden]).toEqual([true, true, true]);
  });

  it("picks another element on the next click", () => {
    document.body.innerHTML = `<button id="a">A</button><button id="b">B</button>`;
    const [a, b] = Array.from(document.querySelectorAll("button"));
    a.getBoundingClientRect = rect(10, 10, 50, 20);
    b.getBoundingClientRect = rect(10, 300, 60, 20);
    pick.start({ strings, target: "To @a · b" });
    under(overlay(), a, document.body, document.documentElement);
    click(a);
    under(overlay(), b, document.body, document.documentElement);
    click(b);
    expect(posts().map((m) => [m.type, m.element.selector, m.rect])).toEqual([
      ["picked", "#a", { x: 10, y: 10, width: 50, height: 20 }],
      ["picked", "#b", { x: 10, y: 300, width: 60, height: 20 }],
    ]);
    expect(placed(q(".picked"))).toEqual(["10px", "300px", "60px", "20px"]);
  });

  it("asks for the box to close on Esc while an element is picked, and keeps picking", () => {
    document.body.innerHTML = `<button>Go</button>`;
    const button = document.querySelector("button")!;
    pick.start({ strings, target: "To @a · b" });
    under(overlay(), button, document.body, document.documentElement);
    click(button);
    expect(key(document.body, "Escape").defaultPrevented).toBe(true);
    expect(types()).toEqual(["picked", "close"]);
    expect(overlay()).not.toBeNull();
    expect(q(".picked").hidden).toBe(true);
    key(document.body, "Escape");
    expect(types()).toEqual(["picked", "close", "ended"]);
    expect(overlay()).toBeNull();
  });

  it("ends picking on Esc while hovering", () => {
    document.body.innerHTML = `<button>Go</button>`;
    pick.start({ strings, target: "To @a · b" });
    expect(key(document.body, "Escape").defaultPrevented).toBe(true);
    expect(overlay()).toBeNull();
    expect(posts()).toEqual([{ type: "ended" }]);
  });

  it("drops the outline and goes back to hovering when the box closes or the comment went out", async () => {
    document.body.innerHTML = `<button>Go</button>`;
    const button = document.querySelector("button")!;
    button.getBoundingClientRect = rect(100, 100, 80, 30);
    pick.start({ strings, target: "To @a · b" });
    under(overlay(), button, document.body, document.documentElement);
    click(button);
    pick.closed();
    expect(q(".picked").hidden).toBe(true);
    await frame();
    expect([q(".highlight").hidden, q(".hint").hidden]).toEqual([false, false]);
    click(button);
    expect(q(".picked").hidden).toBe(false);
    pick.sent();
    expect(q(".picked").hidden).toBe(true);
    key(document.body, "Escape");
    expect(types()).toEqual(["picked", "picked", "ended"]);
  });

  it("posts where the picked element moved to, once a frame at most and only when it moved", async () => {
    document.body.innerHTML = `<button>Go</button>`;
    const button = document.querySelector("button")!;
    button.getBoundingClientRect = rect(100, 300, 80, 30);
    pick.start({ strings, target: "To @a · b" });
    under(overlay(), button, document.body, document.documentElement);
    click(button);
    button.getBoundingClientRect = rect(100, 200, 80, 30);
    for (let i = 0; i < 3; i++) window.dispatchEvent(new Event("scroll"));
    expect(raw).toHaveLength(1);
    await frame();
    expect(posts().slice(1)).toEqual([{ type: "moved", rect: { x: 100, y: 200, width: 80, height: 30 } }]);
    expect(placed(q(".picked"))).toEqual(["100px", "200px", "80px", "30px"]);
    window.dispatchEvent(new Event("scroll"));
    await frame();
    expect(raw).toHaveLength(2);
    button.getBoundingClientRect = rect(60, 200, 80, 30);
    window.dispatchEvent(new Event("resize"));
    await frame();
    button.getBoundingClientRect = rect(60, 240, 80, 30);
    document.body.prepend(document.createElement("p"));
    await frame();
    expect(posts().slice(2)).toEqual([
      { type: "moved", rect: { x: 60, y: 200, width: 80, height: 30 } },
      { type: "moved", rect: { x: 60, y: 240, width: 80, height: 30 } },
    ]);
    pick.closed();
    button.getBoundingClientRect = rect(0, 0, 80, 30);
    window.dispatchEvent(new Event("scroll"));
    await frame();
    expect(raw).toHaveLength(4);
  });

  it("follows the picked element the page moves with no scroll, resize or DOM change, one read a frame", () => {
    const { requestAnimationFrame: raf, cancelAnimationFrame: caf } = globalThis;
    const due = new Map<number, FrameRequestCallback>();
    let ids = 0;
    globalThis.requestAnimationFrame = (callback) => (due.set(++ids, callback), ids);
    globalThis.cancelAnimationFrame = (id) => void due.delete(id);
    const step = () => {
      const callbacks = [...due.values()];
      due.clear();
      for (const callback of callbacks) callback(0);
    };
    try {
      document.body.innerHTML = `<button>Go</button>`;
      const button = document.querySelector("button")!;
      let at = rect(100, 100, 80, 30);
      let reads = 0;
      button.getBoundingClientRect = () => (reads++, at());
      pick.start({ strings, target: "To @a · b" });
      under(overlay(), button, document.body, document.documentElement);
      click(button);
      at = rect(100, 140, 80, 30);
      step();
      expect(posts().slice(1)).toEqual([{ type: "moved", rect: { x: 100, y: 140, width: 80, height: 30 } }]);
      expect(placed(q(".picked"))).toEqual(["100px", "140px", "80px", "30px"]);
      reads = 0;
      step();
      step();
      expect(reads).toBe(2);
      expect(raw).toHaveLength(2);
      window.dispatchEvent(new Event("scroll"));
      reads = 0;
      step();
      expect(reads).toBe(1);
      pick.closed();
      step();
      expect(due.size).toBe(0);
      click(button);
      expect(due.size).toBe(1);
      pick.stop();
      expect(due.size).toBe(0);
    } finally {
      globalThis.requestAnimationFrame = raf;
      globalThis.cancelAnimationFrame = caf;
    }
  });

  it("posts nothing while the picked element is off the page", async () => {
    document.body.innerHTML = `<button class="btn">Go</button>`;
    const button = document.querySelector("button")!;
    button.getBoundingClientRect = rect(100, 100, 100, 40);
    pick.start({ strings, target: "To @a · b" });
    under(overlay(), button, document.body, document.documentElement);
    click(button);
    button.getBoundingClientRect = rect(0, 0, 0, 0);
    document.body.innerHTML = `<button class="btn">Go</button>`;
    window.dispatchEvent(new Event("scroll"));
    await frame();
    expect(types()).toEqual(["picked"]);
    expect(q(".picked").hidden).toBe(true);
  });

  it("keeps the rect within what the app accepts", () => {
    document.body.innerHTML = `<p>long page</p>`;
    document.body.getBoundingClientRect = rect(0, -3_000_000, 800, 3_000_400);
    pick.start({ strings, target: "To @a · b" });
    under(overlay(), document.body, document.documentElement);
    click(document.body);
    expect(posts()[0].rect).toEqual({ x: 0, y: -500_000, width: 800, height: 500_400 });
  });

  it("ignores events the page made up", () => {
    document.body.innerHTML = `<button>Go</button>`;
    const button = document.querySelector("button")!;
    pick.start({ strings, target: "To @a · b" });
    under(overlay(), button, document.body, document.documentElement);
    button.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, detail: 1 }));
    expect(q(".picked").hidden).toBe(true);
    document.body.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    expect(overlay()).not.toBeNull();
    expect(raw).toEqual([]);
    click(button);
    document.body.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    expect(types()).toEqual(["picked"]);
    expect(q(".picked").hidden).toBe(false);
  });

  it("leaves Esc to an input method composing in a page field", () => {
    document.body.innerHTML = `<button>Go</button><input>`;
    const button = document.querySelector("button")!;
    const input = document.querySelector("input")!;
    input.focus();
    pick.start({ strings, target: "To @a · b" });
    expect(key(input, "Escape", { isComposing: true }).defaultPrevented).toBe(false);
    expect(key(input, "Escape", { keyCode: 229 }).defaultPrevented).toBe(false);
    expect(overlay()).not.toBeNull();
    expect(raw).toEqual([]);
    under(overlay(), button, document.body, document.documentElement);
    click(button);
    expect(key(input, "Escape", { isComposing: true }).defaultPrevented).toBe(false);
    expect(types()).toEqual(["picked"]);
    key(input, "Escape");
    key(input, "Escape");
    expect(types()).toEqual(["picked", "close", "ended"]);
  });

  it("follows a page's modal dialog, which makes everything else inert, and comes back after it", async () => {
    document.body.innerHTML = `<button>Go</button><dialog><button>OK</button></dialog>`;
    const button = document.querySelector("button")!;
    const dialog = modal(document.querySelector("dialog")!);
    pick.start({ strings, target: "To @a · b" });
    under(overlay(), button, document.body, document.documentElement);
    click(button);
    for (let i = 0; i < 4; i++) {
      dialog.showModal();
      await frame();
      expect(overlay()!.parentNode).toBe(dialog);
      dialog.close();
      await frame();
      expect(overlay()!.parentNode).toBe(document.documentElement);
    }
    expect(types()).toEqual(["picked"]);
    expect(q(".picked").hidden).toBe(false);
  });

  it("leaves its own overlay out of the HTML of a picked dialog or anything around it", async () => {
    document.body.innerHTML = `<div class="wrap"><dialog><p>Sure?</p></dialog></div>`;
    const wrap = document.querySelector(".wrap")!;
    const dialog = modal(document.querySelector("dialog")!);
    pick.start({ strings, target: "To @a · b" });
    dialog.showModal();
    await frame();
    expect(overlay()!.parentNode).toBe(dialog);
    for (const el of [dialog, wrap]) {
      under(overlay(), el, document.body, document.documentElement);
      click(el);
    }
    const sent = posts().map((m) => m.element.html);
    expect(sent).toHaveLength(2);
    for (const html of sent) {
      expect(html).toContain("Sure?");
      expect(html).not.toContain("sonata-pick");
    }
  });

  it("picks a frame through the shield over it", async () => {
    document.body.innerHTML = `<p>x</p><iframe title="map" srcdoc="<p>map</p>"></iframe>`;
    const iframe = document.querySelector("iframe")!;
    iframe.getBoundingClientRect = rect(10, 10, 200, 100);
    pick.start({ strings, target: "To @a · b" });
    await frame();
    const shield = q(".shield");
    expect(shield.hidden).toBe(false);
    expect(placed(shield)).toEqual(["10px", "10px", "200px", "100px"]);
    under(overlay(), iframe, document.body, document.documentElement);
    let pageSaw = 0;
    const onPage = () => (pageSaw += 1);
    document.addEventListener("click", onPage);
    click(shield, { clientX: 50, clientY: 50 });
    document.removeEventListener("click", onPage);
    expect(pageSaw).toBe(0);
    expect(posts()[0].element.tag).toBe("iframe");
    expect(posts()[0].rect).toEqual({ x: 10, y: 10, width: 200, height: 100 });
  });

  it("stops when the page keeps removing it", async () => {
    pick.start({ strings, target: "To @a · b" });
    for (let i = 0; i < 3; i++) {
      overlay()!.remove();
      await frame();
      expect(overlay()).not.toBeNull();
    }
    overlay()!.remove();
    await frame();
    expect(overlay()).toBeNull();
    expect(posts()).toEqual([{ type: "ended" }]);
  });

  it("picks a 2,000,000-character <pre> quickly and within the limits", () => {
    document.body.innerHTML = "<pre></pre>";
    const pre = document.querySelector("pre")!;
    pre.textContent = "word ".repeat(400_000);
    for (const name of ["outerHTML", "innerHTML", "innerText", "textContent"]) {
      Object.defineProperty(pre, name, {
        get() {
          throw new Error(`read ${name}`);
        },
      });
    }
    pick.start({ strings, target: "To @a · b" });
    under(overlay(), pre, document.body, document.documentElement);
    const started = performance.now();
    click(pre);
    expect(performance.now() - started).toBeLessThan(200);
    const { element } = posts()[0];
    expect([...element.text]).toHaveLength(300);
    expect(element.text.endsWith("…")).toBe(true);
    expect(element.html).toBe("<pre>…</pre>");
  });

  it("sends well-formed text and a tag the app accepts", () => {
    document.body.innerHTML = "";
    const odd = document.createElement("my_widget");
    odd.textContent = "a\uD800b";
    const path = document.createElementNS("http://www.w3.org/2000/svg", "textPath");
    document.body.append(odd, path);
    pick.start({ strings, target: "To @a · b" });
    under(overlay(), odd, document.body, document.documentElement);
    click(odd);
    expect(raw[0]).not.toMatch(/\\ud[89a-f]/i);
    expect(posts()[0].element.text).toBe("a�b");
    expect(posts()[0].element.tag).toBe("element");
    under(overlay(), path, document.body, document.documentElement);
    click(path);
    expect(posts()[1].element.tag).toBe("textpath");
  });
});
