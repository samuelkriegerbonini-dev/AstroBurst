import { describe, expect, it } from "vitest";
import { archiveShortcuts } from "../logic/shortcuts";

describe("archiveShortcuts", () => {
  const keys = archiveShortcuts(false).map((s) => s.keys);

  it.each([
    "/ or Ctrl+F",
    "Enter",
    "Esc",
    "↓ (empty query)",
    "← / → (mission tabs)",
    "F6 / Shift+F6",
    "V",
    "Arrow keys",
    "Home / End",
    "Ctrl+Home / Ctrl+End",
    "PageUp / PageDown",
    "Space",
    "Ctrl+C",
    "Ctrl+Shift+C",
    "T",
    "R",
    "I",
    "Alt+1 / Alt+2 / Alt+3",
    "← / → (previews)",
    "G",
    "↑ / ↓ (Large view)",
    "+ / - / 0 / 1",
  ])("lists %s", (k) => {
    expect(keys).toContain(k);
  });

  it("leaves the workspace switch to the General group", () => {
    expect(keys).not.toContain("Ctrl+1 / Ctrl+2");
  });

  it("names the Command keys on macOS", () => {
    expect(archiveShortcuts(true).map((s) => s.keys)).toEqual(
      expect.arrayContaining(["/ or ⌘F", "⌘Home / ⌘End", "⌘C", "⇧⌘C"]),
    );
  });

  it.each([false, true])("describes every entry and repeats no key (mac %s)", (mac) => {
    const shortcuts = archiveShortcuts(mac);
    expect(shortcuts.every((s) => s.desc.length > 0)).toBe(true);
    expect(new Set(shortcuts.map((s) => s.keys)).size).toBe(shortcuts.length);
  });
});
