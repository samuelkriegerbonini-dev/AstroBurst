import { describe, expect, it, vi } from "vitest";
import { createFocusHandoff } from "../focusHandoff";

function fakeDocument() {
  const body = { focus: vi.fn() } as unknown as HTMLElement;
  return { body, activeElement: body as Element | null };
}

const node = () => ({ focus: vi.fn() }) as unknown as HTMLElement & { focus: ReturnType<typeof vi.fn> };

describe("createFocusHandoff", () => {
  it("moves focus to the arriving button when the leaving one had it and focus fell to the page", () => {
    const doc = fakeDocument();
    const handoff = createFocusHandoff(() => doc);
    const cancel = node();
    const action = node();
    const detach = handoff.leaving(cancel);
    doc.activeElement = cancel;
    detach?.();
    doc.activeElement = doc.body;
    handoff.arriving(action);
    expect(action.focus).toHaveBeenCalledTimes(1);
  });

  it("leaves focus alone when the leaving button did not have it", () => {
    const doc = fakeDocument();
    const handoff = createFocusHandoff(() => doc);
    const action = node();
    handoff.leaving(node())?.();
    handoff.arriving(action);
    expect(action.focus).not.toHaveBeenCalled();
  });

  it("does not take focus from an element the user moved to, and hands off only once", () => {
    const doc = fakeDocument();
    const handoff = createFocusHandoff(() => doc);
    const cancel = node();
    doc.activeElement = cancel;
    handoff.leaving(cancel)?.();
    doc.activeElement = node();
    const action = node();
    handoff.arriving(action);
    doc.activeElement = doc.body;
    handoff.arriving(action);
    expect(action.focus).not.toHaveBeenCalled();
  });

  it("ignores the null React passes when a ref detaches", () => {
    const handoff = createFocusHandoff(() => fakeDocument());
    expect(handoff.leaving(null)).toBeUndefined();
    expect(() => handoff.arriving(null)).not.toThrow();
  });
});
