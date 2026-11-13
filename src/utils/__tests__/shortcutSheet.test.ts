import { describe, expect, it } from "vitest";
import { archiveShortcuts } from "../../components/archive/logic/shortcuts";
import { FRAME_STEP_LARGE } from "../cubeNavigation";
import { shortcutSheet, type ShortcutGroup, type ShortcutRow } from "../shortcutSheet";

function rowsOf(sheet: readonly ShortcutGroup[], id: string): readonly ShortcutRow[] {
  return sheet.find((g) => g.id === id)?.rows ?? [];
}

function keysOf(sheet: readonly ShortcutGroup[], id: string): string[] {
  return rowsOf(sheet, id).map((r) => r.keys);
}

describe("shortcutSheet", () => {
  it("lists Ctrl+Z for undo and Ctrl+Shift+Z or Ctrl+Y for redo under History, for single files", () => {
    const sheet = shortcutSheet(false);
    expect(rowsOf(sheet, "history")).toEqual([
      { keys: "Ctrl+Z", desc: "Undo processing step (single files)" },
      { keys: "Ctrl+Shift+Z / Ctrl+Y", desc: "Redo processing step (single files)" },
    ]);
    expect(rowsOf(sheet, "history").every((r) => r.desc.includes("single files"))).toBe(true);
    expect(rowsOf(shortcutSheet(true), "history").every((r) => r.desc.includes("single files"))).toBe(true);
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

  it("lists the region delete key, which acts only while the image has the focus, under Cube and regions", () => {
    for (const mac of [false, true]) {
      expect(rowsOf(shortcutSheet(mac), "cube-regions")).toContainEqual({ keys: "Delete / Backspace (image focused)", desc: "Delete the selected region" });
    }
  });

  it("lists the frame keys of the focused image next to the spectrum plot keys", () => {
    expect(rowsOf(shortcutSheet(false), "cube-regions").slice(0, 3)).toEqual([
      { keys: "← / → (spectrum plot)", desc: "Previous / next channel of the cube or ramp" },
      { keys: "← / → (focused image)", desc: "Previous / next frame of the cube or ramp on screen" },
      { keys: "Shift+← / →", desc: `Step ${FRAME_STEP_LARGE} channels or frames` },
    ]);
  });

  it("names no Ctrl key on macOS, Archive rows included", () => {
    const mac = shortcutSheet(true);
    expect(mac.flatMap((g) => g.rows.map((r) => r.keys)).filter((k) => k.includes("Ctrl"))).toEqual([]);
    expect(keysOf(mac, "archive")).toEqual(expect.arrayContaining(["/ or ⌘F", "⌘Home / ⌘End", "⌘C", "⇧⌘C"]));
    expect(keysOf(shortcutSheet(false), "archive")).toEqual(expect.arrayContaining(["/ or Ctrl+F", "Ctrl+Home / Ctrl+End", "Ctrl+C", "Ctrl+Shift+C"]));
  });

  it("lists the mode switch once, under the general keys", () => {
    for (const mac of [false, true]) {
      const modeRows = shortcutSheet(mac).flatMap((g) =>
        g.rows.filter((r) => r.desc === "Switch to Workspace / Archive").map((r) => `${g.id}: ${r.keys}`),
      );
      expect(modeRows).toEqual([mac ? "general: ⌘1 / ⌘2" : "general: Ctrl+1 / Ctrl+2"]);
    }
  });

  it("lists no empty group and binds no single-letter key outside the Archive", () => {
    for (const mac of [false, true]) {
      const sheet = shortcutSheet(mac);
      expect(sheet.filter((g) => g.rows.length === 0).map((g) => g.id)).toEqual([]);
      const workspaceKeys = sheet.filter((g) => g.id !== "archive").flatMap((g) => g.rows.map((r) => r.keys));
      expect(workspaceKeys.filter((k) => /^[A-Za-z]$/.test(k))).toEqual([]);
    }
  });

  it("lists the mode switch and the sidebar tool menu with the general keys", () => {
    const general = rowsOf(shortcutSheet(false), "general");
    expect(general).toContainEqual({ keys: "Ctrl+1 / Ctrl+2", desc: "Switch to Workspace / Archive" });
    expect(general).toContainEqual({ keys: "Shift+F10 / right-click (sidebar tool)", desc: "Move or reorder the tool, reset the layout, or open All tools" });
    expect(keysOf(shortcutSheet(true), "general")).toContain("⌘1 / ⌘2");
  });

  it("names the palette entry that opens All tools", () => {
    expect(rowsOf(shortcutSheet(false), "general")).toContainEqual({ keys: "Ctrl+K, then All tools", desc: "Open any tool or choose the sidebar shortcuts" });
    expect(keysOf(shortcutSheet(true), "general")).toContain("⌘K, then All tools");
  });

  it("keeps the existing rows, with the Archive group last", () => {
    const sheet = shortcutSheet(false);
    expect(sheet.map((g) => g.id)).toEqual(["general", "history", "cube-regions", "archive"]);
    expect(sheet[0].title).toBeNull();
    expect(keysOf(sheet, "general")).toEqual([
      "Ctrl+O",
      "Ctrl+K",
      "Shift Shift",
      "Ctrl+1 / Ctrl+2",
      "↑ / ↓",
      "Enter",
      "Shift+F10 / right-click (sidebar tool)",
      "Ctrl+K, then All tools",
      "?",
      "Esc",
    ]);
    expect(keysOf(shortcutSheet(true), "general").slice(0, 2)).toEqual(["⌘O", "⌘K"]);
    expect(rowsOf(sheet, "archive")).toEqual(archiveShortcuts(false));
    expect(rowsOf(shortcutSheet(true), "archive")).toEqual(archiveShortcuts(true));
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
