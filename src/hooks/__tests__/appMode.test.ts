import { describe, expect, it, vi } from "vitest";
import { appModeStore, createAppModeStore, modeFromShortcut } from "../useAppMode";

const key = (over: Partial<Pick<KeyboardEvent, "key" | "code" | "ctrlKey" | "metaKey" | "altKey" | "shiftKey">>) => ({
  key: "",
  code: "",
  ctrlKey: false,
  metaKey: false,
  altKey: false,
  shiftKey: false,
  ...over,
});

describe("appModeStore", () => {
  it("starts in Workspace", () => {
    expect(createAppModeStore().get()).toBe("workspace");
    expect(appModeStore.get()).toBe("workspace");
  });

  it("notifies subscribers on a change and not on a repeat", () => {
    const store = createAppModeStore();
    const cb = vi.fn();
    const off = store.subscribe(cb);
    store.set("archive");
    store.set("archive");
    expect(store.get()).toBe("archive");
    expect(cb).toHaveBeenCalledTimes(1);
    off();
    store.set("workspace");
    expect(cb).toHaveBeenCalledTimes(1);
  });
});

describe("modeFromShortcut", () => {
  it("maps Ctrl+1 and Ctrl+2", () => {
    expect(modeFromShortcut(key({ key: "1", code: "Digit1", ctrlKey: true }))).toBe("workspace");
    expect(modeFromShortcut(key({ key: "2", code: "Digit2", ctrlKey: true }))).toBe("archive");
  });

  it("maps Cmd+2 on macOS", () => {
    expect(modeFromShortcut(key({ key: "2", code: "Digit2", metaKey: true }))).toBe("archive");
  });

  it("reads the physical key when a layout types another character", () => {
    expect(modeFromShortcut(key({ key: "é", code: "Digit2", ctrlKey: true }))).toBe("archive");
    expect(modeFromShortcut(key({ key: "1", code: "Numpad1", ctrlKey: true }))).toBe("workspace");
  });

  it("ignores Shift, Alt, no modifier and other digits", () => {
    expect(modeFromShortcut(key({ key: "@", code: "Digit2", ctrlKey: true, shiftKey: true }))).toBeNull();
    expect(modeFromShortcut(key({ key: "2", code: "Digit2", ctrlKey: true, altKey: true }))).toBeNull();
    expect(modeFromShortcut(key({ key: "2", code: "Digit2" }))).toBeNull();
    expect(modeFromShortcut(key({ key: "3", code: "Digit3", ctrlKey: true }))).toBeNull();
  });
});
