import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createElement, isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { DockOnboardingCard, type DockOnboardingCardProps } from "../dock/DockOnboarding";
import { createDockOnboardingStore, dockOnboardingStore, type OnboardingStorage } from "../dock/dockOnboardingStore";
import { commitDockMove } from "../dock/useDockDrag";
import { dockStore } from "../../hooks/useDockLayout";
import { DEFAULT_DOCK_LAYOUT, dockReducer } from "../../utils/dockLayout";

const KEY = "ab.onboarding.dock.v1";
const TITLE = "Arrange your tool windows";
const STEP_ONE_BODY =
  "Drag any tool icon from a side strip and drop it on the top half of either strip. The tool opens as a full-height column beside the viewer.";
const STEP_TWO_BODY = "Drop it on the bottom half of a strip instead to open it under the viewer. Two bottom tools sit side by side.";
const TIP = "Right-click a tool (or Shift+F10) for Move to…, and Reset Layout puts everything back.";

type CardProps = DockOnboardingCardProps & {
  onNext: ReturnType<typeof vi.fn<() => void>>;
  onBack: ReturnType<typeof vi.fn<() => void>>;
  onClose: ReturnType<typeof vi.fn<() => void>>;
};

function cardProps(over: Partial<DockOnboardingCardProps> = {}): CardProps {
  return {
    step: 0,
    titleId: "onb-title",
    bodyId: "onb-body",
    placement: null,
    onNext: vi.fn<() => void>(),
    onBack: vi.fn<() => void>(),
    onClose: vi.fn<() => void>(),
    ...over,
  } as CardProps;
}

function render(props: DockOnboardingCardProps): string {
  return renderToStaticMarkup(createElement(DockOnboardingCard, props));
}

function plain(html: string): string {
  return html.replace(/<[^>]*>/g, " ").replace(/\s+/g, " ");
}

function buttonTexts(html: string): string[] {
  return [...html.matchAll(/<button\b[^>]*>([\s\S]*?)<\/button>/g)].map((m) => m[1].replace(/<[^>]*>/g, "").trim());
}

type AnyProps = Record<string, unknown> & { children?: ReactNode };

function textOf(node: ReactNode): string {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  if (isValidElement<AnyProps>(node)) return textOf(node.props.children);
  return "";
}

function findElement(node: ReactNode, match: (props: AnyProps) => boolean): ReactElement<AnyProps> {
  const stack: ReactNode[] = [node];
  while (stack.length > 0) {
    const current = stack.pop();
    if (Array.isArray(current)) {
      stack.push(...current);
      continue;
    }
    if (!isValidElement<AnyProps>(current)) continue;
    if (match(current.props)) return current;
    stack.push(current.props.children);
  }
  throw new Error("element not found");
}

function tree(props: DockOnboardingCardProps): ReactElement<AnyProps> {
  return DockOnboardingCard(props) as ReactElement<AnyProps>;
}

function click(props: DockOnboardingCardProps, match: (p: AnyProps) => boolean): void {
  (findElement(tree(props), match).props.onClick as () => void)();
}

function byText(text: string): (p: AnyProps) => boolean {
  return (p) => p.type === "button" && textOf(p.children).trim() === text;
}

function memoryStorage(initial: Record<string, string> = {}) {
  const data = new Map(Object.entries(initial));
  return {
    data,
    getItem: vi.fn((key: string) => data.get(key) ?? null),
    setItem: vi.fn((key: string, value: string) => {
      data.set(key, value);
    }),
  };
}

function throwingStorage(): OnboardingStorage {
  return {
    getItem: () => {
      throw new Error("SecurityError");
    },
    setItem: () => {
      throw new Error("SecurityError");
    },
  };
}

describe("DockOnboardingCard step 1", () => {
  const html = render(cardProps());

  it("is a non-modal dialog labelled by its title", () => {
    expect(html).toMatch(/^<div[^>]*role="dialog"/);
    expect(html).toMatch(/^<div[^>]*aria-modal="false"/);
    expect(html).toMatch(/^<div[^>]*aria-labelledby="onb-title"/);
    expect(html).toMatch(/^<div[^>]*aria-describedby="onb-body"/);
    expect(html).toMatch(new RegExp(`<h2[^>]*id="onb-title"[^>]*>${TITLE}</h2>`));
    expect(html).toMatch(/^<div[^>]*data-dock-onboarding=""/);
    expect(html).toMatch(/^<div[^>]*class="ab-dock-onboarding"/);
  });

  it("shows the first step texts verbatim and the tip", () => {
    const text = plain(html);
    expect(text).toContain("Step 1 of 2");
    expect(text).toContain("Drag vertically");
    expect(html).toMatch(new RegExp(`<p[^>]*id="onb-body"[^>]*>${STEP_ONE_BODY}</p>`));
    expect(text).toContain(TIP);
    expect(text).not.toContain("Drag horizontally");
  });

  it("offers Close and Next only", () => {
    expect(html).toMatch(/<button[^>]*aria-label="Close"/);
    expect(buttonTexts(html)).toEqual(["", "Next"]);
  });

  it("marks the first of two dots and says 1/2", () => {
    expect(html.match(/data-onboarding-dot="/g)).toHaveLength(2);
    expect(html).toMatch(/data-onboarding-dot="active"[^>]*>[\s\S]*data-onboarding-dot="idle"/);
    expect(plain(html)).toContain("1/2");
  });

  it("demonstrates the vertical drop with the real anchor label", () => {
    expect(html).toMatch(/<svg[^>]*data-onboarding-demo="vertical"/);
    expect(html).not.toContain('data-onboarding-demo="horizontal"');
    expect(html).toMatch(/<text[^>]*>Move to Right Top<\/text>/);
    expect(html).toMatch(/class="ab-dock-onboarding-preview"/);
    expect(html).toMatch(/class="ab-dock-onboarding-static-arrow"/);
  });
});

describe("DockOnboardingCard step 2", () => {
  const html = render(cardProps({ step: 1 }));

  it("shows the second step texts verbatim and the tip", () => {
    const text = plain(html);
    expect(text).toContain("Step 2 of 2");
    expect(text).toContain("Drag horizontally");
    expect(html).toMatch(new RegExp(`<p[^>]*id="onb-body"[^>]*>${STEP_TWO_BODY}</p>`));
    expect(text).toContain(TIP);
    expect(text).toContain(TITLE);
  });

  it("offers Close, Back and Got it", () => {
    expect(html).toMatch(/<button[^>]*aria-label="Close"/);
    expect(buttonTexts(html)).toEqual(["", "Back", "Got it"]);
  });

  it("marks the second dot and says 2/2", () => {
    expect(html).toMatch(/data-onboarding-dot="idle"[^>]*>[\s\S]*data-onboarding-dot="active"/);
    expect(plain(html)).toContain("2/2");
  });

  it("demonstrates the horizontal drop with the real anchor label", () => {
    expect(html).toMatch(/<svg[^>]*data-onboarding-demo="horizontal"/);
    expect(html).not.toContain('data-onboarding-demo="vertical"');
    expect(html).toMatch(/<text[^>]*>Move to Bottom Right<\/text>/);
  });
});

describe("DockOnboardingCard placement and actions", () => {
  it("applies the computed placement and arrow", () => {
    const html = render(cardProps({ placement: { left: 824, top: 116, width: 360, maxHeight: 568, arrow: "right" } }));
    expect(html).toMatch(/^<div[^>]*style="left:824px;top:116px;width:360px;max-height:568px"/);
    expect(html).toMatch(/^<div[^>]*data-arrow="right"/);
    const narrow = render(cardProps({ placement: { left: 216, top: 96, width: 348, maxHeight: 468, arrow: "none" } }));
    expect(narrow).toMatch(/^<div[^>]*data-arrow="none"/);
  });

  it("wires Next, Back, Got it and the close button", () => {
    const first = cardProps();
    click(first, byText("Next"));
    expect(first.onNext).toHaveBeenCalledTimes(1);
    click(first, (p) => p["aria-label"] === "Close");
    expect(first.onClose).toHaveBeenCalledTimes(1);
    const second = cardProps({ step: 1 });
    click(second, byText("Back"));
    expect(second.onBack).toHaveBeenCalledTimes(1);
    click(second, byText("Got it"));
    expect(second.onClose).toHaveBeenCalledTimes(1);
  });

  it("closes on Escape pressed inside the card and leaves other keys alone", () => {
    const props = cardProps();
    const dialog = findElement(tree(props), (p) => p.role === "dialog");
    const onKeyDown = dialog.props.onKeyDown as (e: unknown) => void;
    const other = { key: "Enter", preventDefault: vi.fn(), stopPropagation: vi.fn() };
    onKeyDown(other);
    expect(props.onClose).not.toHaveBeenCalled();
    expect(other.preventDefault).not.toHaveBeenCalled();
    const escape = { key: "Escape", preventDefault: vi.fn(), stopPropagation: vi.fn() };
    onKeyDown(escape);
    expect(props.onClose).toHaveBeenCalledTimes(1);
    expect(escape.preventDefault).toHaveBeenCalled();
    expect(escape.stopPropagation).toHaveBeenCalled();
  });
});

describe("dock onboarding session store", () => {
  it("opens at step 1 on first launch and writes done when dismissed", () => {
    const storage = memoryStorage();
    const store = createDockOnboardingStore(() => storage);
    store.autoShow();
    expect(store.get()).toEqual({ open: true, step: 0 });
    store.setStep(1);
    expect(store.get()).toEqual({ open: true, step: 1 });
    store.dismiss();
    expect(store.get().open).toBe(false);
    expect(storage.data.get(KEY)).toBe("done");
    store.autoShow();
    expect(store.get().open).toBe(false);
  });

  it("stays closed once done, but the re-open command shows step 1 again", () => {
    const storage = memoryStorage({ [KEY]: "done" });
    const store = createDockOnboardingStore(() => storage);
    store.autoShow();
    expect(store.get().open).toBe(false);
    store.show();
    expect(store.get()).toEqual({ open: true, step: 0 });
    store.setStep(1);
    store.show();
    expect(store.get()).toEqual({ open: true, step: 0 });
    expect(storage.setItem).not.toHaveBeenCalled();
  });

  it("shows once per session when storage throws, and never throws", () => {
    const store = createDockOnboardingStore(throwingStorage);
    expect(() => store.autoShow()).not.toThrow();
    expect(store.get().open).toBe(true);
    expect(() => store.dismiss()).not.toThrow();
    expect(store.get().open).toBe(false);
    store.autoShow();
    expect(store.get().open).toBe(false);
  });

  it("notifies subscribers only on a change", () => {
    const store = createDockOnboardingStore(() => memoryStorage());
    const listener = vi.fn();
    store.subscribe(listener);
    store.show();
    store.show();
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it("closes and writes done after a move to another anchor, not after a reorder", () => {
    const storage = memoryStorage();
    const store = createDockOnboardingStore(() => storage);
    store.autoShow();
    const reordered = dockReducer(DEFAULT_DOCK_LAYOUT, { type: "move", tool: "image", anchor: "right-top", index: 0 });
    store.afterMove(DEFAULT_DOCK_LAYOUT, reordered, "image");
    expect(store.get().open).toBe(true);
    expect(storage.setItem).not.toHaveBeenCalled();
    const moved = dockReducer(DEFAULT_DOCK_LAYOUT, { type: "move", tool: "image", anchor: "left-bottom" });
    store.afterMove(DEFAULT_DOCK_LAYOUT, moved, "image");
    expect(store.get().open).toBe(false);
    expect(storage.data.get(KEY)).toBe("done");
  });

  it("does not auto-show later in the session after a move dismissed it first", () => {
    const store = createDockOnboardingStore(throwingStorage);
    const moved = dockReducer(DEFAULT_DOCK_LAYOUT, { type: "move", tool: "config", anchor: "left-top" });
    store.afterMove(DEFAULT_DOCK_LAYOUT, moved, "config");
    store.autoShow();
    expect(store.get().open).toBe(false);
  });
});

describe("dock move path", () => {
  let storage: ReturnType<typeof memoryStorage>;

  beforeEach(() => {
    storage = memoryStorage();
    vi.stubGlobal("localStorage", storage);
    dockStore.dispatch({ type: "reset" });
    dockOnboardingStore.show();
  });

  afterEach(() => {
    dockOnboardingStore.dismiss();
    dockStore.dispatch({ type: "reset" });
    vi.unstubAllGlobals();
  });

  it("marks onboarding done and hides the card when a tool changes anchor", () => {
    expect(dockOnboardingStore.get().open).toBe(true);
    expect(commitDockMove("image", "right-bottom")).toBe(true);
    expect(dockStore.get().anchors["right-bottom"]).toContain("image");
    expect(storage.data.get(KEY)).toBe("done");
    expect(dockOnboardingStore.get().open).toBe(false);
  });

  it("keeps the card for a reorder inside the same anchor and for a no-op move", () => {
    expect(commitDockMove("image", "right-top", 0)).toBe(true);
    expect(commitDockMove("image", "right-top", 0)).toBe(false);
    expect(storage.setItem.mock.calls.some(([key]) => key === KEY)).toBe(false);
    expect(dockOnboardingStore.get().open).toBe(true);
  });
});
