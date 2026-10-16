import { describe, expect, it } from "vitest";
import { largeKeyCommand, previewsKeyCommand, resultsKeyCommand } from "../logic/detailKeys";

function key(k: string, mods: Partial<{ ctrlKey: boolean; metaKey: boolean; altKey: boolean; shiftKey: boolean }> = {}) {
  return { key: k, ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, ...mods };
}

describe("largeKeyCommand", () => {
  it.each([
    ["Escape", "close"],
    ["ArrowLeft", "prev-image"],
    ["ArrowRight", "next-image"],
    ["ArrowUp", "prev-dataset"],
    ["ArrowDown", "next-dataset"],
    ["g", "next-group"],
    ["G", "next-group"],
    ["+", "zoom-in"],
    ["=", "zoom-in"],
    ["-", "zoom-out"],
    ["0", "zoom-fit"],
    ["1", "zoom-actual"],
    ["i", "toggle-rail"],
    ["r", "retry"],
  ])("maps %s to %s", (k, command) => {
    expect(largeKeyCommand(key(k))).toBe(command);
  });

  it("takes Shift for the + key and leaves other combinations alone", () => {
    expect(largeKeyCommand(key("+", { shiftKey: true }))).toBe("zoom-in");
    expect(largeKeyCommand(key("ArrowLeft", { altKey: true }))).toBeNull();
    expect(largeKeyCommand(key("g", { ctrlKey: true }))).toBeNull();
    expect(largeKeyCommand(key("x"))).toBeNull();
  });

  it("sends Ctrl+F to the query field instead of the browser find bar", () => {
    expect(largeKeyCommand(key("f", { ctrlKey: true }))).toBe("focus-query");
    expect(largeKeyCommand(key("F", { metaKey: true }))).toBe("focus-query");
  });
});

describe("previewsKeyCommand", () => {
  it("walks images, cycles groups and opens the Large view", () => {
    expect(previewsKeyCommand(key("ArrowLeft"), { inInput: false })).toBe("prev-image");
    expect(previewsKeyCommand(key("ArrowRight"), { inInput: false })).toBe("next-image");
    expect(previewsKeyCommand(key("G"), { inInput: false })).toBe("next-group");
    expect(previewsKeyCommand(key(" "), { inInput: false })).toBe("open-large");
  });

  it("ignores modified keys and typing", () => {
    expect(previewsKeyCommand(key("ArrowLeft", { ctrlKey: true }), { inInput: false })).toBeNull();
    expect(previewsKeyCommand(key("g"), { inInput: true })).toBeNull();
  });
});

describe("resultsKeyCommand", () => {
  it("moves into the detail on Enter and opens the Large view on Space", () => {
    expect(resultsKeyCommand(key("Enter"))).toBe("open-detail");
    expect(resultsKeyCommand(key(" "))).toBe("open-large");
    expect(resultsKeyCommand(key("Enter", { shiftKey: true }))).toBeNull();
    expect(resultsKeyCommand(key(" ", { ctrlKey: true }))).toBeNull();
    expect(resultsKeyCommand(key("a"))).toBeNull();
  });
});
