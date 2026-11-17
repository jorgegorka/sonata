import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { TERMINAL_THEME } from "./terminal";

const read = (path: string) => readFileSync(new URL(path, import.meta.url), "utf8");
const styles = read("./styles.css");
const pickbox = read("./pickbox/pickbox.css");

type Tokens = Map<string, string>;
interface Sheet {
  light: Tokens;
  dark: Tokens;
  rest: string;
}

const LIGHT_BLOCK = /^:root \{([^}]*)\}/m;
const DARK_BLOCK = /@media \(prefers-color-scheme: dark\) \{\s*:root \{([^}]*)\}\s*\}/;
const COLOR = /#[0-9a-f]{3,8}\b|\b(?:rgba?|hsla?)\(/i;

function tokens(block: string): Tokens {
  return new Map([...block.matchAll(/(--[\w-]+):\s*([^;]+);/g)].map(([, name, value]) => [name, value.trim().toLowerCase()]));
}

function parse(css: string): Sheet {
  const light = css.match(LIGHT_BLOCK);
  const dark = css.match(DARK_BLOCK);
  if (!light || !dark) throw new Error("token blocks not found");
  const rest = css.replace(light[0], "").replace(dark[0], "").replace(/\/\*[\s\S]*?\*\//g, "");
  return { light: tokens(light[1]), dark: tokens(dark[1]), rest };
}

function hardcodedColors(rest: string, allowed: (selector: string, declaration: string) => boolean): string[] {
  const found: string[] = [];
  for (const [, rawSelector, body] of rest.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    const selector = rawSelector.trim();
    for (const declaration of body.split(";").map((d) => d.trim())) {
      if (COLOR.test(declaration) && !allowed(selector, declaration)) found.push(`${selector} { ${declaration} }`);
    }
  }
  return found;
}

const NEW_TOKENS = ["--ok", "--busy", "--alert", "--waits", "--console-bg", "--console-text"];

describe("theme", () => {
  it("keeps every color of the app in the token blocks", () => {
    const allowed = (selector: string, declaration: string) =>
      // Each bot's avatar sets its own background inline; the custom swatch is a rainbow on purpose.
      (selector === ".avatar" && declaration === "color: #fff") ||
      (selector === ".swatch.custom" && declaration.startsWith("background: conic-gradient("));
    expect(hardcodedColors(parse(styles).rest, allowed)).toEqual([]);
  });

  it("keeps every color of the element picker in its token blocks", () => {
    expect(hardcodedColors(parse(pickbox).rest, () => false)).toEqual([]);
  });

  it("defines every token for both light and dark", () => {
    const { light, dark } = parse(styles);
    expect([...light.keys()]).toEqual(expect.arrayContaining(NEW_TOKENS));
    expect([...dark.keys()]).toEqual(expect.arrayContaining(NEW_TOKENS));
    expect([...dark.keys()].sort()).toEqual([...light.keys()].sort());
  });

  it("paints the terminal with the console colors", () => {
    const { light, dark } = parse(styles);
    for (const scheme of [light, dark]) {
      expect(TERMINAL_THEME.background).toBe(scheme.get("--console-bg"));
      expect(TERMINAL_THEME.foreground).toBe(scheme.get("--console-text"));
    }
  });

  it("gives the element picker the app's colors", () => {
    const app = parse(styles);
    const pick = parse(pickbox);
    // The picker floats over web pages, so its surface is the app's raised one: white in light mode, the panel in dark.
    const source = { light: (name: string) => name, dark: (name: string) => (name === "--bg" ? "--panel" : name) };
    for (const scheme of ["light", "dark"] as const) {
      for (const [name, value] of pick[scheme]) {
        expect(value, `${scheme} ${name}`).toBe(app[scheme].get(source[scheme](name)));
      }
    }
  });
});
