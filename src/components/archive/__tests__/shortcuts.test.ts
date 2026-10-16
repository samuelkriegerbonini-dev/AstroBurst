import { describe, expect, it } from "vitest";
import { ARCHIVE_SHORTCUTS } from "../logic/shortcuts";

describe("ARCHIVE_SHORTCUTS", () => {
  const keys = ARCHIVE_SHORTCUTS.map((s) => s.keys);

  it.each([
    "Ctrl+1 / Ctrl+2",
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

  it("describes every entry and repeats no key", () => {
    expect(ARCHIVE_SHORTCUTS.every((s) => s.desc.length > 0)).toBe(true);
    expect(new Set(keys).size).toBe(keys.length);
  });
});
