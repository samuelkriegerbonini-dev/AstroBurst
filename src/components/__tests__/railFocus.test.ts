import { describe, expect, it, vi } from "vitest";
import { focusRailTool, railToolTarget } from "../dock/railFocus";

const FILES_RAIL_BUTTON = '[data-dock-strip] [data-tool-id="files"]';
const RIGHT_RAIL_TOOLS_TRIGGER = '[data-dock-strip="right"] [data-tools-trigger]';

function fakeElement(rendered: boolean) {
  return { getClientRects: () => (rendered ? [{}] : []), focus: vi.fn(), closest: () => ({}), scrollIntoView: vi.fn() };
}

function fakeRoot(found: Record<string, object[]>): ParentNode {
  return {
    querySelectorAll: (selector: string) => found[selector] ?? [],
    querySelector: (selector: string) => found[selector]?.[0] ?? null,
  } as unknown as ParentNode;
}

describe("focusRailTool", () => {
  it("focuses the rendered rail button of the tool", () => {
    const hidden = fakeElement(false);
    const shown = fakeElement(true);
    const root = fakeRoot({ [FILES_RAIL_BUTTON]: [hidden, shown] });
    expect(railToolTarget("files", root)).toBe(shown);
    focusRailTool("files", root);
    expect(shown.focus).toHaveBeenCalledWith({ preventScroll: true });
    expect(hidden.focus).not.toHaveBeenCalled();
  });

  it("falls back to the All tools button of the right rail", () => {
    const trigger = fakeElement(true);
    const root = fakeRoot({ [RIGHT_RAIL_TOOLS_TRIGGER]: [trigger] });
    expect(railToolTarget("files", root)).toBe(trigger);
    focusRailTool("files", root);
    expect(trigger.focus).toHaveBeenCalledWith({ preventScroll: true });
  });

  it("does nothing when neither exists", () => {
    const root = fakeRoot({});
    expect(railToolTarget("files", root)).toBeNull();
    expect(() => focusRailTool("files", root)).not.toThrow();
  });
});
