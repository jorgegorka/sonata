import { describe, expect, it } from "vitest";
import { linkAction } from "./links";

describe("linkAction", () => {
  const plain = { metaKey: false };
  const command = { metaKey: true };

  it("opens web pages in the panel", () => {
    expect(linkAction("https://gitlab.com/sonata/sonata/-/merge_requests/1", plain)).toBe("inline");
    expect(linkAction("http://localhost:3000/", plain)).toBe("inline");
  });

  it("hands a ⌘-click to the system browser", () => {
    expect(linkAction("https://example.com/", command)).toBe("external");
  });

  it("always hands mail links to the system", () => {
    expect(linkAction("mailto:a@b.c", plain)).toBe("external");
    expect(linkAction("mailto:a@b.c", command)).toBe("external");
  });

  it("ignores anything else", () => {
    for (const href of ["javascript:alert(1)", "file:///etc/passwd", "/relative/path", "#anchor", "", undefined]) {
      expect(linkAction(href, plain), String(href)).toBe("ignore");
      expect(linkAction(href, command), String(href)).toBe("ignore");
    }
  });
});
