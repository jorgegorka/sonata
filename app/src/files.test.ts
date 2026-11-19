/// <reference types="vite/client" />
import { describe, expect, it } from "vitest";
import attachmentsRs from "../../crates/sonata-core/src/attachments.rs?raw";
import { RUNNABLE_EXTENSIONS, addFiles, fileName, opensPrograms } from "./files";

describe("files", () => {
  it("knows which files would run a program when opened", () => {
    expect(opensPrograms("setup.command")).toBe(true);
    expect(opensPrograms("Installer.PKG")).toBe(true);
    expect(opensPrograms("report.md")).toBe(false);
    expect(opensPrograms(".sh")).toBe(false);
    expect(opensPrograms("site.webloc")).toBe(true);
    for (const name of ["install.deb", "app.desktop", "Sonata.AppImage", "install.rpm"]) expect(opensPrograms(name)).toBe(true);
  });

  it("lists the same extensions as the Rust side, sorted", () => {
    const list = attachmentsRs.match(/RUNNABLE_EXTENSIONS: &\[&str\] = &\[([^\]]*)\]/);
    expect(list).not.toBeNull();
    const rust = [...list![1].matchAll(/"([^"]+)"/g)].map((m) => m[1]);
    expect(rust.length).toBeGreaterThan(0);
    expect(RUNNABLE_EXTENSIONS).toEqual(rust);
    expect([...RUNNABLE_EXTENSIONS].sort()).toEqual(RUNNABLE_EXTENSIONS);
  });

  it("names a file by the last part of its path", () => {
    expect(fileName("/Users/me/Notas de reunión.md")).toBe("Notas de reunión.md");
    expect(fileName("/Users/me/folder/")).toBe("folder");
  });

  it("adds new paths once, in order", () => {
    expect(addFiles(["/a"], ["/b", "/a", "/b", "/c"])).toEqual(["/a", "/b", "/c"]);
  });
});
