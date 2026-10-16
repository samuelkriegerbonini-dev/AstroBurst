import { describe, expect, it } from "vitest";
import { archiveKeyCommand, isTextEntry, type ArchiveKeyEvent } from "../logic/archiveKeys";

function key(k: string, over: Partial<ArchiveKeyEvent> = {}): ArchiveKeyEvent {
  return { key: k, code: "", ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, ...over };
}

describe("archiveKeyCommand", () => {
  it("focuses the query on / outside inputs and on Ctrl+F or Cmd+F anywhere", () => {
    expect(archiveKeyCommand(key("/"), { inInput: false })).toBe("focus-query");
    expect(archiveKeyCommand(key("/", { shiftKey: true }), { inInput: false })).toBe("focus-query");
    expect(archiveKeyCommand(key("/"), { inInput: true })).toBeNull();
    expect(archiveKeyCommand(key("f", { ctrlKey: true, code: "KeyF" }), { inInput: true })).toBe("focus-query");
    expect(archiveKeyCommand(key("F", { metaKey: true, code: "KeyF" }), { inInput: false })).toBe("focus-query");
    expect(archiveKeyCommand(key("f", { ctrlKey: true, shiftKey: true, code: "KeyF" }), { inInput: false })).toBeNull();
  });

  it("cycles regions with F6 and Shift+F6, also from an input", () => {
    expect(archiveKeyCommand(key("F6"), { inInput: true })).toBe("next-region");
    expect(archiveKeyCommand(key("F6", { shiftKey: true }), { inInput: false })).toBe("previous-region");
  });

  it("maps the single-letter keys only outside inputs and without modifiers", () => {
    expect(archiveKeyCommand(key("v"), { inInput: false })).toBe("toggle-view");
    expect(archiveKeyCommand(key("V", { shiftKey: true }), { inInput: false })).toBe("toggle-view");
    expect(archiveKeyCommand(key("r"), { inInput: false })).toBe("retry");
    expect(archiveKeyCommand(key("i"), { inInput: false })).toBe("toggle-detail");
    expect(archiveKeyCommand(key("v"), { inInput: true })).toBeNull();
    expect(archiveKeyCommand(key("v", { ctrlKey: true }), { inInput: false })).toBeNull();
    expect(archiveKeyCommand(key("r", { altKey: true }), { inInput: false })).toBeNull();
  });

  it("switches detail tabs with Alt+1, Alt+2 and Alt+3 by physical key", () => {
    expect(archiveKeyCommand(key("1", { altKey: true, code: "Digit1" }), { inInput: false })).toBe("tab-previews");
    expect(archiveKeyCommand(key("¡", { altKey: true, code: "Digit1" }), { inInput: true })).toBe("tab-previews");
    expect(archiveKeyCommand(key("2", { altKey: true, code: "Digit2" }), { inInput: false })).toBe("tab-metadata");
    expect(archiveKeyCommand(key("3", { altKey: true, code: "Numpad3" }), { inInput: false })).toBe("tab-products");
    expect(archiveKeyCommand(key("2", { altKey: true, ctrlKey: true, code: "Digit2" }), { inInput: false })).toBeNull();
  });

  it("ignores the app-level keys", () => {
    expect(archiveKeyCommand(key("k", { ctrlKey: true, code: "KeyK" }), { inInput: false })).toBeNull();
    expect(archiveKeyCommand(key("2", { ctrlKey: true, code: "Digit2" }), { inInput: false })).toBeNull();
    expect(archiveKeyCommand(key("?"), { inInput: false })).toBeNull();
    expect(archiveKeyCommand(key("Escape"), { inInput: false })).toBeNull();
  });
});

describe("isTextEntry", () => {
  it("treats text inputs, textareas, selects and editable content as typing targets", () => {
    expect(isTextEntry({ tagName: "INPUT", type: "text" })).toBe(true);
    expect(isTextEntry({ tagName: "INPUT", type: "date" })).toBe(true);
    expect(isTextEntry({ tagName: "TEXTAREA" })).toBe(true);
    expect(isTextEntry({ tagName: "SELECT" })).toBe(true);
    expect(isTextEntry({ tagName: "DIV", isContentEditable: true })).toBe(true);
  });

  it("does not treat buttons, checkboxes, cards or nothing as typing targets", () => {
    expect(isTextEntry({ tagName: "BUTTON" })).toBe(false);
    expect(isTextEntry({ tagName: "INPUT", type: "checkbox" })).toBe(false);
    expect(isTextEntry({ tagName: "DIV" })).toBe(false);
    expect(isTextEntry(null)).toBe(false);
  });
});
