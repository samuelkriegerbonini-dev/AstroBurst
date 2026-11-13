import { describe, expect, it } from "vitest";
import { regionKeyAction, type RegionKeyContext } from "../regionKeys";

const ON_IMAGE: RegionKeyContext = { editable: false, inViewer: true, inModal: false, draft: null, hasSelection: false, tool: "none" };
const ON_PANEL: RegionKeyContext = { ...ON_IMAGE, inViewer: false };

describe("regionKeyAction with Delete and Backspace", () => {
  it("deletes the selected region while the image has the focus", () => {
    expect(regionKeyAction("Delete", { ...ON_IMAGE, hasSelection: true })).toBe("delete-selected");
    expect(regionKeyAction("Backspace", { ...ON_IMAGE, hasSelection: true, tool: "select" })).toBe("delete-selected");
  });

  it("deletes nothing from the Files list, a dock button or a panel", () => {
    expect(regionKeyAction("Delete", { ...ON_PANEL, hasSelection: true })).toBeNull();
    expect(regionKeyAction("Backspace", { ...ON_PANEL, hasSelection: true })).toBeNull();
  });

  it("deletes nothing in a text field, in a dialog or without a selection", () => {
    expect(regionKeyAction("Backspace", { ...ON_IMAGE, hasSelection: true, editable: true })).toBeNull();
    expect(regionKeyAction("Delete", { ...ON_IMAGE, hasSelection: true, inModal: true })).toBeNull();
    expect(regionKeyAction("Delete", ON_IMAGE)).toBeNull();
  });
});

describe("regionKeyAction with Escape", () => {
  it("clears the draft first, then the selection, then the region tool", () => {
    expect(regionKeyAction("Escape", { ...ON_IMAGE, draft: "polygon", hasSelection: true, tool: "polygon" })).toBe("clear-draft");
    expect(regionKeyAction("Escape", { ...ON_IMAGE, hasSelection: true, tool: "select" })).toBe("deselect");
    expect(regionKeyAction("Escape", { ...ON_IMAGE, tool: "circle" })).toBe("tool-off");
    expect(regionKeyAction("Escape", ON_IMAGE)).toBeNull();
  });

  it("keeps acting from outside the image, as before", () => {
    expect(regionKeyAction("Escape", { ...ON_PANEL, draft: "box", tool: "box" })).toBe("clear-draft");
    expect(regionKeyAction("Escape", { ...ON_PANEL, tool: "select" })).toBe("tool-off");
  });

  it("leaves the draft, the selection and the tool as they were when Esc closes a dialog", () => {
    const states: Partial<RegionKeyContext>[] = [
      { draft: "polygon", tool: "polygon" },
      { hasSelection: true, tool: "select" },
      { tool: "polygon" },
    ];
    for (const state of states) {
      expect(regionKeyAction("Escape", { ...ON_PANEL, ...state, inModal: true })).toBeNull();
      expect(regionKeyAction("Escape", { ...ON_IMAGE, ...state, inModal: true })).toBeNull();
    }
  });

  it("does nothing in a text field", () => {
    expect(regionKeyAction("Escape", { ...ON_IMAGE, draft: "polygon", tool: "polygon", editable: true })).toBeNull();
  });
});

describe("regionKeyAction with Enter", () => {
  it("closes a polygon draft and leaves any other draft alone", () => {
    expect(regionKeyAction("Enter", { ...ON_IMAGE, draft: "polygon", tool: "polygon" })).toBe("close-polygon");
    expect(regionKeyAction("Enter", { ...ON_IMAGE, draft: "circle", tool: "circle" })).toBeNull();
    expect(regionKeyAction("Enter", { ...ON_IMAGE, hasSelection: true, tool: "select" })).toBeNull();
  });

  it("does not close the polygon from a dialog or a text field", () => {
    expect(regionKeyAction("Enter", { ...ON_IMAGE, draft: "polygon", tool: "polygon", inModal: true })).toBeNull();
    expect(regionKeyAction("Enter", { ...ON_IMAGE, draft: "polygon", tool: "polygon", editable: true })).toBeNull();
  });
});

describe("regionKeyAction with other keys", () => {
  it("maps no other key", () => {
    for (const key of ["a", "ArrowLeft", "Tab", " ", "?"]) {
      expect(regionKeyAction(key, { ...ON_IMAGE, draft: "polygon", hasSelection: true, tool: "polygon" })).toBeNull();
    }
  });
});
