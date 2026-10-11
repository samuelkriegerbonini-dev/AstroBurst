import { describe, it, expect } from "vitest";
import {
  discardNoticesAfterReset,
  nextDiscardNotices,
  STACK_INPUTS_CHANGED,
  stackAllNext,
  stackRunFinish,
  stackStateAfterReset,
  type StackRunStart,
} from "../stackRun";

const started: StackRunStart = { binId: "r", token: 7, generation: 4, files: ["/a.fits", "/b.fits", "/c.fits"] };

describe("stackRunFinish", () => {
  it("stores the result when the generation and the effective files are unchanged", () => {
    expect(stackRunFinish(started, { generation: 4, files: ["/a.fits", "/b.fits", "/c.fits"] })).toEqual({
      store: true,
      notice: null,
    });
  });

  it("discards the result with the notice when a reset bumped the generation, even with the same files", () => {
    expect(stackRunFinish(started, { generation: 5, files: ["/a.fits", "/b.fits", "/c.fits"] })).toEqual({
      store: false,
      notice: STACK_INPUTS_CHANGED,
    });
  });

  it("discards the result when the bin's effective files changed", () => {
    const changed = { store: false, notice: STACK_INPUTS_CHANGED };
    expect(stackRunFinish(started, { generation: 4, files: ["/a.fits", "/b.fits"] })).toEqual(changed);
    expect(stackRunFinish(started, { generation: 4, files: ["/a.fits", "/b.fits", "/d.fits"] })).toEqual(changed);
    expect(stackRunFinish(started, { generation: 4, files: ["/b.fits", "/a.fits", "/c.fits"] })).toEqual(changed);
    expect(stackRunFinish(started, { generation: 4, files: [] })).toEqual(changed);
  });

  it("uses the pinned notice text", () => {
    expect(STACK_INPUTS_CHANGED).toBe("Inputs changed while stacking; result discarded");
  });
});

describe("stackStateAfterReset", () => {
  it("bumps the generation and cancels only a running stack", () => {
    expect(stackStateAfterReset(4, true)).toEqual({ generation: 5, cancel: true });
    expect(stackStateAfterReset(4, false)).toEqual({ generation: 5, cancel: false });
  });
});

describe("stackAllNext", () => {
  const sameWizard = { generation: 2, live: 2 };

  it("stops Stack All when the user cancelled", () => {
    expect(stackAllNext("cancelled", sameWizard)).toBe("stop");
  });

  it("moves on to the next bin after a discarded, failed or stored bin", () => {
    expect(stackAllNext("discarded", sameWizard)).toBe("continue");
    expect(stackAllNext("failed", sameWizard)).toBe("continue");
    expect(stackAllNext("done", sameWizard)).toBe("continue");
  });

  it("stops Stack All after a wizard Reset", () => {
    expect(stackAllNext("discarded", { generation: 2, live: 3 })).toBe("stop");
    expect(stackAllNext("done", { generation: 2, live: 3 })).toBe("stop");
  });
});

describe("discardNoticesAfterReset", () => {
  it("leaves no discard notice after a wizard Reset", () => {
    expect(discardNoticesAfterReset()).toEqual({});
  });
});

describe("nextDiscardNotices", () => {
  const discard = { store: false, notice: STACK_INPUTS_CHANGED };
  const stored = { store: true, notice: null };
  const live = { generation: 4, live: 4 };

  it("sets the bin's notice on a discard and keeps the other bins", () => {
    expect(nextDiscardNotices({ g: "older" }, "r", discard, live)).toEqual({ g: "older", r: STACK_INPUTS_CHANGED });
  });

  it("clears the bin's notice when a stack is stored and keeps the other bins", () => {
    expect(nextDiscardNotices({ r: STACK_INPUTS_CHANGED, g: "older" }, "r", stored, live)).toEqual({ g: "older" });
  });

  it("leaves the notices alone for a discard without a notice", () => {
    expect(nextDiscardNotices({ g: "older" }, "r", { store: false, notice: null }, live)).toEqual({ g: "older" });
  });

  it("ignores the outcome of a run started before a Reset", () => {
    const stale = { generation: 3, live: 4 };
    expect(nextDiscardNotices({}, "r", discard, stale)).toEqual({});
    expect(nextDiscardNotices({ r: STACK_INPUTS_CHANGED }, "r", stored, stale)).toEqual({ r: STACK_INPUTS_CHANGED });
  });

  it("never mutates its input", () => {
    const notices = Object.freeze({ r: STACK_INPUTS_CHANGED, g: "older" });
    const cleared = nextDiscardNotices(notices, "r", stored, live);
    const set = nextDiscardNotices(notices, "b", discard, live);
    expect(notices).toEqual({ r: STACK_INPUTS_CHANGED, g: "older" });
    expect(cleared).not.toBe(notices);
    expect(set).not.toBe(notices);
  });
});
