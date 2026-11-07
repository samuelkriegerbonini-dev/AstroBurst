import { describe, expect, it } from "vitest";
import {
  NEW_BATCH_CONFIRM_DELAY_MS,
  NO_OVERLAYS,
  newBatchConfirmAccepted,
  newBatchPaletteEntry,
  overlaysAfter,
  shellKeyAction,
  shiftReleaseAction,
  type ShellKeyContext,
  type ShellKeyEvent,
  type ShellOverlays,
} from "../shellKeys";

const SHEET: ShellOverlays = { paletteOpen: false, shortcutsOpen: true };
const PALETTE: ShellOverlays = { paletteOpen: true, shortcutsOpen: false };
const OUTSIDE: ShellKeyContext = { inInput: false };
const IN_INPUT: ShellKeyContext = { inInput: true };

function key(name: string, mods: Partial<ShellKeyEvent> = {}): ShellKeyEvent {
  return { key: name, ctrlKey: false, metaKey: false, ...mods };
}

function press(overlays: ShellOverlays, e: ShellKeyEvent, ctx: ShellKeyContext = OUTSIDE): ShellOverlays {
  const action = shellKeyAction(e, ctx);
  if (action === null || action === "open-files") return overlays;
  return overlaysAfter(overlays, action);
}

describe("shell keys", () => {
  it("closes the shortcut sheet when Ctrl+K opens the palette, so the two never stack", () => {
    expect(press(SHEET, key("k", { ctrlKey: true }))).toEqual(PALETTE);
    expect(press(SHEET, key("K", { metaKey: true }))).toEqual(PALETTE);
    expect(press(SHEET, key("k", { ctrlKey: true }), IN_INPUT)).toEqual(PALETTE);
    expect(press(PALETTE, key("k", { ctrlKey: true }))).toEqual(NO_OVERLAYS);
  });

  it("closes the palette when ? opens the shortcut sheet, and ignores ? typed in a text field", () => {
    expect(press(PALETTE, key("?"))).toEqual(SHEET);
    expect(press(SHEET, key("?"))).toEqual(NO_OVERLAYS);
    expect(shellKeyAction(key("?"), IN_INPUT)).toBeNull();
  });

  it("closes both overlays on Escape outside a text field", () => {
    expect(press(PALETTE, key("Escape"))).toEqual(NO_OVERLAYS);
    expect(press(SHEET, key("Escape"))).toEqual(NO_OVERLAYS);
    expect(press({ paletteOpen: true, shortcutsOpen: true }, key("Escape"))).toEqual(NO_OVERLAYS);
    expect(shellKeyAction(key("Escape"), IN_INPUT)).toBeNull();
  });

  it("keeps the same state on Escape when nothing is open, so the shell does not re-render", () => {
    const closed: ShellOverlays = { paletteOpen: false, shortcutsOpen: false };
    expect(overlaysAfter(closed, "close-overlays")).toBe(closed);
  });

  it("treats a double Shift like Ctrl+K", () => {
    const ctrlK = shellKeyAction(key("k", { ctrlKey: true }), OUTSIDE);
    expect(shiftReleaseAction(1000, 0)).toBeNull();
    expect(shiftReleaseAction(1200, 1000)).toBe(ctrlK);
    expect(shiftReleaseAction(1400, 1000)).toBeNull();
    const doubleShift = shiftReleaseAction(1200, 1000);
    expect(doubleShift && overlaysAfter(SHEET, doubleShift)).toEqual(PALETTE);
  });

  it("opens files on Ctrl+O even from a text field and leaves plain letters alone", () => {
    expect(shellKeyAction(key("o", { ctrlKey: true }), IN_INPUT)).toBe("open-files");
    expect(shellKeyAction(key("O", { metaKey: true }), OUTSIDE)).toBe("open-files");
    expect(shellKeyAction(key("k"), OUTSIDE)).toBeNull();
    expect(shellKeyAction(key("Shift"), OUTSIDE)).toBeNull();
  });
});

describe("New Batch in the palette", () => {
  it("asks first when processed files would be lost, and the next pick confirms the discard", () => {
    expect(newBatchPaletteEntry(3, false)).toEqual({ label: "New Batch (discard processed files)", hint: "asks to confirm", step: "arm" });
    expect(newBatchPaletteEntry(3, true)).toMatchObject({ label: "Discard 3 processed files", step: "confirm" });
    expect(newBatchPaletteEntry(1, true).label).toBe("Discard 1 processed file");
  });

  it("keeps matching the query that found New Batch once it asks to confirm", () => {
    expect(newBatchPaletteEntry(3, true).keywords).toContain("New Batch");
  });

  it("starts the new batch at once when no processed file would be lost", () => {
    expect(newBatchPaletteEntry(0, false)).toEqual({ label: "New Batch", step: "discard" });
    expect(newBatchPaletteEntry(0, true)).toEqual({ label: "New Batch", step: "discard" });
  });

  it("gives a screen reader a prompt for the relabelled option, since the selected option keeps its id", () => {
    expect(newBatchPaletteEntry(3, true).prompt).toBe("Discard 3 processed files? Press Enter again to confirm, or Escape to cancel.");
    expect(newBatchPaletteEntry(1, true).prompt).toContain("Discard 1 processed file?");
    expect(newBatchPaletteEntry(3, false).prompt).toBeUndefined();
    expect(newBatchPaletteEntry(0, true).prompt).toBeUndefined();
  });
});

describe("Confirming New Batch from the palette", () => {
  it("ignores the second click of a double click and a quick second Enter", () => {
    expect(newBatchConfirmAccepted(0, false)).toBe(false);
    expect(newBatchConfirmAccepted(150, false)).toBe(false);
    expect(newBatchConfirmAccepted(NEW_BATCH_CONFIRM_DELAY_MS - 1, false)).toBe(false);
  });

  it("accepts a deliberate pick once the confirm has been on screen for 400 to 500 ms", () => {
    expect(NEW_BATCH_CONFIRM_DELAY_MS).toBeGreaterThanOrEqual(400);
    expect(NEW_BATCH_CONFIRM_DELAY_MS).toBeLessThanOrEqual(500);
    expect(newBatchConfirmAccepted(NEW_BATCH_CONFIRM_DELAY_MS, false)).toBe(true);
    expect(newBatchConfirmAccepted(2000, false)).toBe(true);
  });

  it("ignores an Enter held past the key-repeat delay, however long it is held", () => {
    expect(newBatchConfirmAccepted(600, true)).toBe(false);
    expect(newBatchConfirmAccepted(10_000, true)).toBe(false);
  });
});
