import { describe, expect, it } from "vitest";
import { ARCHIVE_SHORTCUTS } from "../../components/archive/logic/shortcuts";
import { FRAME_STEP_LARGE } from "../cubeNavigation";
import { shortcutSheet, type ShortcutGroup, type ShortcutRow } from "../shortcutSheet";

function rowsOf(sheet: readonly ShortcutGroup[], id: string): readonly ShortcutRow[] {
  return sheet.find((g) => g.id === id)?.rows ?? [];
}

function keysOf(sheet: readonly ShortcutGroup[], id: string): string[] {
  return rowsOf(sheet, id).map((r) => r.keys);
}

describe("shortcutSheet", () => {
  it("lists Ctrl+Z for undo and Ctrl+Shift+Z or Ctrl+Y for redo under History", () => {
    const sheet = shortcutSheet(false);
    expect(rowsOf(sheet, "history")).toEqual([
      { keys: "Ctrl+Z", desc: "Undo processing step" },
      { keys: "Ctrl+Shift+Z / Ctrl+Y", desc: "Redo processing step" },
    ]);
    expect(sheet.find((g) => g.id === "history")?.title).toBe("History");
  });

  it("uses the Command key on macOS, where Ctrl+Y does not redo", () => {
    expect(keysOf(shortcutSheet(true), "history")).toEqual(["⌘Z", "⇧⌘Z"]);
  });

  it("lists cube channel stepping, the channel jump and polygon closing under Cube and regions", () => {
    const sheet = shortcutSheet(false);
    const rows = rowsOf(sheet, "cube-regions");
    const desc = (keys: string) => rows.find((r) => r.keys === keys)?.desc ?? "";
    expect(desc("← / → (spectrum plot)")).toContain("channel");
    expect(desc("Shift+← / →")).toContain(`${FRAME_STEP_LARGE} channels`);
    expect(desc("Double-click (spectrum plot)")).toContain("channel");
    expect(desc("Enter / double-click (polygon tool)")).toContain("Close the polygon");
    expect(sheet.find((g) => g.id === "cube-regions")?.title).toBe("Cube and regions");
  });

  it("reserves an empty Viewer group and binds no single-letter key outside the Archive", () => {
    const sheet = shortcutSheet(false);
    const viewer = sheet.find((g) => g.id === "viewer");
    expect(viewer?.title).toBe("Viewer");
    expect(viewer?.rows).toEqual([]);
    const workspaceKeys = sheet.filter((g) => g.id !== "archive").flatMap((g) => g.rows.map((r) => r.keys));
    expect(workspaceKeys.filter((k) => /^[A-Za-z]$/.test(k))).toEqual([]);
  });

  it("keeps the existing rows, with the Archive group last", () => {
    const sheet = shortcutSheet(false);
    expect(sheet.map((g) => g.id)).toEqual(["general", "history", "cube-regions", "viewer", "archive"]);
    expect(sheet[0].title).toBeNull();
    expect(keysOf(sheet, "general")).toEqual(["Ctrl+O", "Ctrl+K", "Shift Shift", "↑ / ↓", "Enter", "?", "Esc"]);
    expect(keysOf(shortcutSheet(true), "general").slice(0, 2)).toEqual(["⌘O", "⌘K"]);
    expect(rowsOf(sheet, "archive")).toBe(ARCHIVE_SHORTCUTS);
  });

  it("describes every row and repeats no key inside a group", () => {
    for (const mac of [false, true]) {
      for (const g of shortcutSheet(mac)) {
        const keys = g.rows.map((r) => r.keys);
        expect(g.rows.every((r) => r.desc.length > 0)).toBe(true);
        expect(new Set(keys).size).toBe(keys.length);
      }
    }
  });
});
