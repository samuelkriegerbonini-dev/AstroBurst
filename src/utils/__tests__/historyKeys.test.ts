import { describe, expect, it } from "vitest";
import { historyKeyAction, isMacPlatform, type HistoryKeyContext, type HistoryKeyEvent } from "../historyKeys";

const WORKSPACE: HistoryKeyContext = { inInput: false, menuOpen: false, mode: "workspace", compositeMode: false, mac: false };
const MAC: HistoryKeyContext = { ...WORKSPACE, mac: true };

function key(k: string, mods: Partial<HistoryKeyEvent> = {}): HistoryKeyEvent {
  return { key: k, code: `Key${k.toUpperCase()}`, ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, ...mods };
}

describe("historyKeyAction", () => {
  it("historyKeyAction_ignores_inputs_menus_and_archive_mode", () => {
    const undo = key("z", { ctrlKey: true });
    expect(historyKeyAction(undo, WORKSPACE)).toBe("undo");
    expect(historyKeyAction(undo, { ...WORKSPACE, inInput: true })).toBeNull();
    expect(historyKeyAction(undo, { ...WORKSPACE, menuOpen: true })).toBeNull();
    expect(historyKeyAction(undo, { ...WORKSPACE, mode: "archive" })).toBeNull();
  });

  it("leaves Ctrl+Z alone while a composite is on screen, so a hidden channel file never moves", () => {
    expect(historyKeyAction(key("z", { ctrlKey: true }), { ...WORKSPACE, compositeMode: true })).toBeNull();
    expect(historyKeyAction(key("y", { ctrlKey: true }), { ...WORKSPACE, compositeMode: true })).toBeNull();
    expect(historyKeyAction(key("z", { metaKey: true, shiftKey: true }), { ...MAC, compositeMode: true })).toBeNull();
  });

  it("maps Ctrl+Z to undo and Ctrl+Shift+Z or Ctrl+Y to redo", () => {
    expect(historyKeyAction(key("z", { ctrlKey: true }), WORKSPACE)).toBe("undo");
    expect(historyKeyAction(key("Z", { ctrlKey: true, shiftKey: true }), WORKSPACE)).toBe("redo");
    expect(historyKeyAction(key("y", { ctrlKey: true }), WORKSPACE)).toBe("redo");
    expect(historyKeyAction(key("y", { ctrlKey: true, shiftKey: true }), WORKSPACE)).toBeNull();
  });

  it("uses Cmd on a Mac and leaves Ctrl+Z there to the system", () => {
    expect(historyKeyAction(key("z", { metaKey: true }), MAC)).toBe("undo");
    expect(historyKeyAction(key("z", { metaKey: true, shiftKey: true }), MAC)).toBe("redo");
    expect(historyKeyAction(key("z", { ctrlKey: true }), MAC)).toBeNull();
    expect(historyKeyAction(key("y", { metaKey: true }), MAC)).toBeNull();
    expect(historyKeyAction(key("z", { metaKey: true }), WORKSPACE)).toBeNull();
  });

  it("ignores Alt chords, bare letters and other keys", () => {
    expect(historyKeyAction(key("z", { ctrlKey: true, altKey: true }), WORKSPACE)).toBeNull();
    expect(historyKeyAction(key("z"), WORKSPACE)).toBeNull();
    expect(historyKeyAction(key("x", { ctrlKey: true }), WORKSPACE)).toBeNull();
    expect(historyKeyAction(key("z", { ctrlKey: true, metaKey: true }), WORKSPACE)).toBeNull();
  });

  it("reads the physical key when the layout reports no letter", () => {
    expect(historyKeyAction({ ...key("z", { ctrlKey: true }), key: "Unidentified" }, WORKSPACE)).toBe("undo");
    expect(historyKeyAction({ ...key("y", { ctrlKey: true }), key: "Process" }, WORKSPACE)).toBe("redo");
  });

  it("detects a Mac from the platform string", () => {
    expect(isMacPlatform("MacIntel")).toBe(true);
    expect(isMacPlatform("macOS")).toBe(true);
    expect(isMacPlatform("Win32")).toBe(false);
    expect(isMacPlatform("Linux x86_64")).toBe(false);
  });
});
