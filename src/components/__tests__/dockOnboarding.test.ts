import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createElement, isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import DockOnboarding, { DockOnboardingCard, DockOnboardingPopup, type DockOnboardingCardProps } from "../dock/DockOnboarding";
import { DockTipCard, type DockTipCardProps } from "../dock/DockTip";
import { createDockOnboardingStore, dockOnboardingStore, type OnboardingStorage } from "../dock/dockOnboardingStore";
import { onboardingAnchorReady, rightRailToolsTrigger } from "../dock/dockGeometry";
import { tipAnchorSelector } from "../dock/tipAnchor";
import { commitDockMove } from "../dock/useDockDrag";
import { hooks } from "../../test/hookRuntime";
import { dockStore } from "../../hooks/useDockLayout";
import { DEFAULT_DOCK_LAYOUT, dockReducer } from "../../utils/dockLayout";
import { DOCK_TIPS, onboardingPlacement, viewerToolbarBottom, type OnboardingToolbarHost } from "../../utils/dockOnboarding";

vi.mock("react", async (original) => {
  const { reactWithFakeHooks } = await import("../../test/hookRuntime");
  return reactWithFakeHooks(await original<Record<string, unknown>>());
});

vi.mock("react-dom", async (original) => ({
  ...(await original<Record<string, unknown>>()),
  createPortal: (node: unknown) => node,
}));

const KEY = "ab.onboarding.dock.v1";
const TOGGLE_KEY = "ab.onboarding.dockToggle.v1";
const SETTINGS_KEY = "ab.onboarding.settings.v1";
const TOGGLE_BODY = "Click any tool in the sidebar to open it, and click it again to close it.";
const SETTINGS_BODY =
  "Click Settings for the astrometry.net API key, plate solving, catalogs, archive downloads and output storage. Click it again to close it.";
const TITLE = "Arrange your sidebar tools";
const STEP_ONE_BODY =
  "Drag any tool icon from a sidebar and drop it on the top half of either sidebar. The tool opens as a full-height column beside the viewer.";
const STEP_TWO_BODY = "Drop it on the bottom half of a sidebar instead to open it under the viewer. Two bottom tools sit side by side.";
const TIP = "Right-click a tool (or Shift+F10) for Move to…, and Reset layout puts everything back.";

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

  it("draws the All tools trigger at the top of the right rail in both demos", () => {
    for (const demo of [html, render(cardProps({ step: 1 }))]) {
      expect(demo.match(/class="ab-dock-onboarding-demo-trigger"/g)).toHaveLength(1);
      expect(demo).toMatch(/<rect class="ab-dock-onboarding-demo-trigger" x="228.5" y="5"/);
      expect(demo).not.toMatch(/<rect class="ab-dock-onboarding-slot" x="228.5" y="5"/);
    }
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

  it("starts the card below the viewer toolbar rows it measured", () => {
    const viewer = { left: 300, top: 60, width: 900, height: 700 };
    const host: OnboardingToolbarHost = {
      getBoundingClientRect: () => ({ top: 60, bottom: 760 }),
      querySelector: () => ({ getBoundingClientRect: () => ({ top: 92, bottom: 760 }) }),
      querySelectorAll: () => [{ getBoundingClientRect: () => ({ top: 126, bottom: 158 }) }],
    };
    const html = render(cardProps({ placement: onboardingPlacement(viewer, { width: 1600, height: 900 }, viewerToolbarBottom(host)) }));
    const top = Number(/^<div[^>]*style="[^"]*top:(\d+)px/.exec(html)?.[1]);
    expect(top).toBeGreaterThanOrEqual(158 + 16);
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
    expect(store.get()).toEqual({ card: "tour", step: 0 });
    store.setStep(1);
    expect(store.get()).toEqual({ card: "tour", step: 1 });
    store.dismiss();
    expect(store.get().card).toBeNull();
    expect(storage.data.get(KEY)).toBe("done");
    store.autoShow();
    expect(store.get().card).toBeNull();
  });

  it("stays closed once done, but the re-open command shows step 1 again", () => {
    const storage = memoryStorage({ [KEY]: "done" });
    const store = createDockOnboardingStore(() => storage);
    store.autoShow();
    expect(store.get().card).toBeNull();
    store.show();
    expect(store.get()).toEqual({ card: "tour", step: 0 });
    store.setStep(1);
    store.show();
    expect(store.get()).toEqual({ card: "tour", step: 0 });
    expect(storage.setItem).not.toHaveBeenCalled();
  });

  it("shows once per session when storage throws, and never throws", () => {
    const store = createDockOnboardingStore(throwingStorage);
    expect(() => store.autoShow()).not.toThrow();
    expect(store.get().card).toBe("tour");
    expect(() => store.dismiss()).not.toThrow();
    expect(store.get().card).toBeNull();
    store.autoShow();
    expect(store.get().card).toBeNull();
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
    expect(store.get().card).toBe("tour");
    expect(storage.setItem).not.toHaveBeenCalled();
    const moved = dockReducer(DEFAULT_DOCK_LAYOUT, { type: "move", tool: "image", anchor: "left-bottom" });
    store.afterMove(DEFAULT_DOCK_LAYOUT, moved, "image");
    expect(store.get().card).toBeNull();
    expect(storage.data.get(KEY)).toBe("done");
  });

  it("does not auto-show later in the session after a move dismissed it first", () => {
    const store = createDockOnboardingStore(throwingStorage);
    const moved = dockReducer(DEFAULT_DOCK_LAYOUT, { type: "move", tool: "config", anchor: "left-top" });
    store.afterMove(DEFAULT_DOCK_LAYOUT, moved, "config");
    store.autoShow();
    expect(store.get().card).toBeNull();
  });
});

function manualSchedule() {
  const pending = new Map<number, { run: () => void; ms: number }>();
  let seq = 0;
  return {
    schedule(run: () => void, ms: number): () => void {
      seq += 1;
      const id = seq;
      pending.set(id, { run, ms });
      return () => {
        pending.delete(id);
      };
    },
    delays: () => [...pending.values()].map((p) => p.ms),
    flush() {
      const due = [...pending.values()];
      pending.clear();
      for (const p of due) p.run();
    },
  };
}

function queueStore(storage: OnboardingStorage = memoryStorage()) {
  const timers = manualSchedule();
  const store = createDockOnboardingStore(() => storage, timers.schedule);
  return { store, timers };
}

describe("dock onboarding queue", () => {
  it("gives a new user without a file the drag tour, then the toggle tip and the settings tip once a file loads", () => {
    const storage = memoryStorage();
    const { store, timers } = queueStore(storage);
    expect(store.get().card).toBeNull();
    store.autoShow();
    expect(store.get()).toEqual({ card: "tour", step: 0 });
    store.setStep(1);
    store.dismiss();
    expect(store.get().card).toBeNull();
    expect(storage.data.get(KEY)).toBe("done");
    expect(timers.delays()).toEqual([600]);
    timers.flush();
    expect(store.get().card).toBeNull();
    store.setReady("toggle", true);
    store.setReady("settings", true);
    expect(store.get()).toEqual({ card: "toggle", step: 0 });
    store.dismiss();
    expect(storage.data.get(TOGGLE_KEY)).toBe("done");
    timers.flush();
    expect(store.get()).toEqual({ card: "settings", step: 0 });
    store.dismiss();
    expect(storage.data.get(SETTINGS_KEY)).toBe("done");
    timers.flush();
    expect(store.get().card).toBeNull();
  });

  it("shows the toggle tip, the settings tip and the drag tour in that order when all are ready", () => {
    const { store, timers } = queueStore();
    store.setReady("toggle", true);
    store.setReady("settings", true);
    store.autoShow();
    const seen: (string | null)[] = [];
    for (let i = 0; i < 4; i++) {
      seen.push(store.get().card);
      store.dismiss();
      timers.flush();
    }
    expect(seen).toEqual(["toggle", "settings", "tour", null]);
  });

  it("keeps the open card when another becomes ready, and shows that one after the delay", () => {
    const { store, timers } = queueStore();
    store.autoShow();
    expect(store.get().card).toBe("tour");
    store.setReady("toggle", true);
    expect(store.get().card).toBe("tour");
    store.dismiss();
    expect(store.get().card).toBeNull();
    timers.flush();
    expect(store.get().card).toBe("toggle");
  });

  it("gives users who finished the drag tour the two new tips once", () => {
    const storage = memoryStorage({ [KEY]: "done" });
    const { store, timers } = queueStore(storage);
    store.setReady("toggle", true);
    store.setReady("settings", true);
    store.autoShow();
    expect(store.get().card).toBe("toggle");
    store.dismiss();
    timers.flush();
    expect(store.get().card).toBe("settings");
    store.dismiss();
    timers.flush();
    expect(store.get().card).toBeNull();
    const next = queueStore(storage).store;
    next.setReady("toggle", true);
    next.setReady("settings", true);
    next.autoShow();
    expect(next.get().card).toBeNull();
  });

  it("shows nothing before the first auto-show, and nothing new while a dock drag is in progress", () => {
    const { store, timers } = queueStore();
    store.setReady("toggle", true);
    store.setReady("settings", true);
    expect(store.get().card).toBeNull();
    store.setDragging(true);
    store.autoShow();
    expect(store.get().card).toBeNull();
    store.setDragging(false);
    expect(store.get().card).toBe("toggle");
    store.setDragging(true);
    expect(store.get().card).toBe("toggle");
    store.dismiss();
    timers.flush();
    expect(store.get().card).toBeNull();
    store.setDragging(false);
    expect(store.get().card).toBe("settings");
  });

  it("Skip tips closes the card and marks every tip and the drag tour done", () => {
    const storage = memoryStorage();
    const { store, timers } = queueStore(storage);
    store.setReady("toggle", true);
    store.setReady("settings", true);
    store.autoShow();
    store.skipAll();
    expect(store.get().card).toBeNull();
    expect(storage.data.get(TOGGLE_KEY)).toBe("done");
    expect(storage.data.get(SETTINGS_KEY)).toBe("done");
    expect(storage.data.get(KEY)).toBe("done");
    timers.flush();
    store.setReady("toggle", false);
    store.setReady("toggle", true);
    expect(store.get().card).toBeNull();
  });

  it("marks the toggle tip done when any strip tool is toggled while it is shown", () => {
    const storage = memoryStorage();
    const { store, timers } = queueStore(storage);
    store.setReady("toggle", true);
    store.setReady("settings", true);
    store.autoShow();
    store.stripToggled("files");
    expect(store.get().card).toBeNull();
    expect(storage.data.get(TOGGLE_KEY)).toBe("done");
    timers.flush();
    expect(store.get().card).toBe("settings");
  });

  it("marks the settings tip done only when Config is toggled from the strip or Settings opens while it is shown", () => {
    const storage = memoryStorage();
    const { store, timers } = queueStore(storage);
    store.setReady("settings", true);
    store.autoShow();
    store.stripToggled("files");
    expect(store.get().card).toBe("settings");
    store.stripToggled("config");
    expect(store.get().card).toBeNull();
    expect(storage.data.get(SETTINGS_KEY)).toBe("done");
    store.show();
    expect(store.get().card).toBe("settings");
    store.setSettingsOpen(true);
    expect(store.get().card).toBeNull();
    timers.flush();
    expect(store.get().card).toBe("tour");
  });

  it("does not count Settings opened under another card, nor Settings still open when its tip shows", () => {
    const { store, timers } = queueStore();
    store.setReady("toggle", true);
    store.setReady("settings", true);
    store.autoShow();
    store.setSettingsOpen(true);
    expect(store.get().card).toBe("toggle");
    store.dismiss();
    timers.flush();
    expect(store.get().card).toBe("settings");
    store.setSettingsOpen(true);
    expect(store.get().card).toBe("settings");
    store.setSettingsOpen(false);
    expect(store.get().card).toBe("settings");
    store.setSettingsOpen(true);
    expect(store.get().card).toBeNull();
  });

  it("hides a tip whose button is gone without marking it done, and shows it again when it is back", () => {
    const storage = memoryStorage({ [KEY]: "done" });
    const { store, timers } = queueStore(storage);
    store.setReady("toggle", true);
    store.autoShow();
    expect(store.get().card).toBe("toggle");
    store.setReady("toggle", false);
    expect(store.get().card).toBeNull();
    expect(storage.data.has(TOGGLE_KEY)).toBe(false);
    timers.flush();
    expect(store.get().card).toBeNull();
    store.setReady("toggle", true);
    expect(store.get().card).toBe("toggle");
  });

  it("hides the drag tour while the viewer is hidden and brings it back", () => {
    const { store, timers } = queueStore();
    store.autoShow();
    store.setReady("tour", false);
    expect(store.get().card).toBeNull();
    timers.flush();
    store.setReady("tour", true);
    expect(store.get()).toEqual({ card: "tour", step: 0 });
  });

  it("replays the whole sequence from the first card that is ready", () => {
    const storage = memoryStorage({ [KEY]: "done", [TOGGLE_KEY]: "done", [SETTINGS_KEY]: "done" });
    const { store, timers } = queueStore(storage);
    store.setReady("settings", true);
    store.autoShow();
    expect(store.get().card).toBeNull();
    store.show();
    expect(store.get().card).toBe("settings");
    store.dismiss();
    timers.flush();
    expect(store.get().card).toBe("tour");
    store.dismiss();
    timers.flush();
    expect(store.get().card).toBeNull();
    store.setReady("toggle", true);
    expect(store.get().card).toBe("toggle");
    store.show();
    expect(store.get().card).toBe("toggle");
    store.dismiss();
    store.show();
    expect(store.get().card).toBe("toggle");
    expect(timers.delays()).toEqual([]);
  });

  it("keeps the tip steps at zero and an open tip through a real move", () => {
    const storage = memoryStorage();
    const { store } = queueStore(storage);
    store.setReady("toggle", true);
    store.autoShow();
    store.setStep(1);
    expect(store.get()).toEqual({ card: "toggle", step: 0 });
    const moved = dockReducer(DEFAULT_DOCK_LAYOUT, { type: "move", tool: "compose", anchor: "right-bottom" });
    store.afterMove(DEFAULT_DOCK_LAYOUT, moved, "compose");
    expect(store.get().card).toBe("toggle");
    expect(storage.data.get(KEY)).toBe("done");
  });

  it("shows every card once per session when storage throws", () => {
    const { store, timers } = queueStore(throwingStorage());
    store.setReady("toggle", true);
    store.setReady("settings", true);
    store.autoShow();
    const seen: (string | null)[] = [];
    for (let i = 0; i < 4; i++) {
      seen.push(store.get().card);
      expect(() => store.dismiss()).not.toThrow();
      timers.flush();
    }
    expect(seen).toEqual(["toggle", "settings", "tour", null]);
    store.autoShow();
    expect(store.get().card).toBeNull();
  });
});

type TipProps = DockTipCardProps & {
  onGotIt: ReturnType<typeof vi.fn<() => void>>;
  onSkip: ReturnType<typeof vi.fn<() => void>>;
};

function tipProps(over: Partial<DockTipCardProps> = {}): TipProps {
  return {
    tip: DOCK_TIPS.toggle,
    titleId: "tip-title",
    bodyId: "tip-body",
    placement: null,
    onGotIt: vi.fn<() => void>(),
    onSkip: vi.fn<() => void>(),
    ...over,
  } as TipProps;
}

function renderTip(props: DockTipCardProps): string {
  return renderToStaticMarkup(createElement(DockTipCard, props));
}

function tipTree(props: DockTipCardProps): ReactElement<AnyProps> {
  return DockTipCard(props) as ReactElement<AnyProps>;
}

describe("DockTipCard", () => {
  it("is a non-modal dialog labelled by its title and described by its body", () => {
    const html = renderTip(tipProps());
    expect(html).toMatch(/^<div[^>]*role="dialog"/);
    expect(html).toMatch(/^<div[^>]*aria-modal="false"/);
    expect(html).toMatch(/^<div[^>]*aria-labelledby="tip-title"/);
    expect(html).toMatch(/^<div[^>]*aria-describedby="tip-body"/);
    expect(html).toMatch(/^<div[^>]*data-dock-tip="toggle"/);
    expect(html).toMatch(/^<div[^>]*class="ab-dock-onboarding ab-dock-tip"/);
    expect(html).toMatch(/<h2[^>]*id="tip-title"[^>]*>Open and close tools<\/h2>/);
    expect(html).toMatch(new RegExp(`<p[^>]*id="tip-body"[^>]*>${TOGGLE_BODY}</p>`));
  });

  it("shows the settings tip texts", () => {
    const html = renderTip(tipProps({ tip: DOCK_TIPS.settings }));
    expect(html).toMatch(/^<div[^>]*data-dock-tip="settings"/);
    expect(html).toMatch(/<h2[^>]*id="tip-title"[^>]*>Settings<\/h2>/);
    expect(html).toMatch(new RegExp(`<p[^>]*id="tip-body"[^>]*>${SETTINGS_BODY}</p>`));
  });

  it("offers Skip tips and Got it, with Got it as the primary button", () => {
    const html = renderTip(tipProps());
    expect(buttonTexts(html)).toEqual(["Skip tips", "Got it"]);
    expect(html).toMatch(/<button[^>]*class="ab-dock-onboarding-btn ab-dock-onboarding-btn-primary"[^>]*>Got it<\/button>/);
    expect(html).toMatch(/<button[^>]*class="ab-dock-onboarding-btn"[^>]*>Skip tips<\/button>/);
    expect(html).not.toMatch(/aria-label="Close"/);
  });

  it("applies the placement and points its arrow at the anchor side", () => {
    const left = renderTip(tipProps({ placement: { left: 51, top: 764, width: 280, maxHeight: 868, arrow: "left", arrowY: 74 } }));
    expect(left).toMatch(/^<div[^>]*style="left:51px;top:764px;width:280px;max-height:868px;--ab-tip-arrow-y:74px"/);
    expect(left).toMatch(/^<div[^>]*data-arrow="left"/);
    const right = renderTip(tipProps({ tip: DOCK_TIPS.settings, placement: { left: 1269, top: 658, width: 280, maxHeight: 868, arrow: "right", arrowY: 60 } }));
    expect(right).toMatch(/^<div[^>]*data-arrow="right"/);
    expect(right).toMatch(/^<div[^>]*style="[^"]*--ab-tip-arrow-y:60px"/);
  });

  it("stays hidden with no arrow until it has been placed", () => {
    const html = renderTip(tipProps());
    expect(html).toMatch(/^<div[^>]*data-arrow="none"/);
    expect(html).toMatch(/^<div[^>]*style="[^"]*visibility:hidden"/);
  });

  it("wires Got it and Skip tips", () => {
    const props = tipProps();
    (findElement(tipTree(props), byText("Got it")).props.onClick as () => void)();
    expect(props.onGotIt).toHaveBeenCalledTimes(1);
    expect(props.onSkip).not.toHaveBeenCalled();
    (findElement(tipTree(props), byText("Skip tips")).props.onClick as () => void)();
    expect(props.onSkip).toHaveBeenCalledTimes(1);
  });

  it("closes as done on Escape pressed inside the card and leaves other keys alone", () => {
    const props = tipProps();
    const onKeyDown = findElement(tipTree(props), (p) => p.role === "dialog").props.onKeyDown as (e: unknown) => void;
    const other = { key: "Tab", preventDefault: vi.fn(), stopPropagation: vi.fn() };
    onKeyDown(other);
    expect(props.onGotIt).not.toHaveBeenCalled();
    expect(other.preventDefault).not.toHaveBeenCalled();
    const escape = { key: "Escape", preventDefault: vi.fn(), stopPropagation: vi.fn() };
    onKeyDown(escape);
    expect(props.onGotIt).toHaveBeenCalledTimes(1);
    expect(escape.preventDefault).toHaveBeenCalled();
    expect(escape.stopPropagation).toHaveBeenCalled();
  });
});

describe("onboardingAnchorReady", () => {
  function anchor({ disabled = false, inert = false }: { disabled?: boolean; inert?: boolean } = {}) {
    return {
      getAttribute: (name: string) => (name === "aria-disabled" && disabled ? "true" : null),
      closest: (selector: string) => (selector === "[inert]" && inert ? {} : null),
    };
  }

  it("is ready for an enabled anchor on screen", () => {
    expect(onboardingAnchorReady(anchor())).toBe(true);
  });

  it("does not offer a tip for a shortcut hidden through the tool catalog", () => {
    expect(onboardingAnchorReady({ ...anchor(), getClientRects: () => [] })).toBe(false);
    expect(onboardingAnchorReady({ ...anchor(), getClientRects: () => [{}] })).toBe(true);
  });

  it("is not ready when the anchor is missing, disabled or inside a hidden inert layer", () => {
    expect(onboardingAnchorReady(null)).toBe(false);
    expect(onboardingAnchorReady(undefined)).toBe(false);
    expect(onboardingAnchorReady(anchor({ disabled: true }))).toBe(false);
    expect(onboardingAnchorReady(anchor({ inert: true }))).toBe(false);
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
    expect(dockOnboardingStore.get().card).toBe("tour");
    expect(commitDockMove("image", "right-bottom")).toBe(true);
    expect(dockStore.get().anchors["right-bottom"]).toContain("image");
    expect(storage.data.get(KEY)).toBe("done");
    expect(dockOnboardingStore.get().card).toBeNull();
  });

  it("keeps the card for a reorder inside the same anchor and for a no-op move", () => {
    expect(commitDockMove("image", "right-top", 0)).toBe(true);
    expect(commitDockMove("image", "right-top", 0)).toBe(false);
    expect(storage.setItem.mock.calls.some(([key]) => key === KEY)).toBe(false);
    expect(dockOnboardingStore.get().card).toBe("tour");
  });
});

interface FakeTarget {
  calls: string[];
  getAttribute(name: string): string | null;
  closest(selector: string): unknown;
  getClientRects(): unknown[];
  focus(): void;
  scrollIntoView(): void;
}

function fakeTarget({ rendered = true, disabled = () => false }: { rendered?: boolean; disabled?: () => boolean } = {}): FakeTarget {
  const calls: string[] = [];
  return {
    calls,
    getAttribute: (name) => (name === "aria-disabled" && disabled() ? "true" : null),
    closest: (selector) => (selector === ".ab-dock-strip" ? {} : null),
    getClientRects: () => (rendered ? [{}] : []),
    focus: () => void calls.push("focus"),
    scrollIntoView: () => void calls.push("scroll"),
  };
}

function fakeViewer() {
  return {
    ...fakeTarget(),
    getBoundingClientRect: () => ({ left: 0, top: 0, width: 900, height: 600, bottom: 600 }),
    querySelector: () => null,
    querySelectorAll: () => [],
  };
}

function resetOnboardingSingleton(): void {
  dockOnboardingStore.skipAll();
  hooks.mount();
  dockOnboardingStore.setReady("toggle", false);
  dockOnboardingStore.setReady("settings", false);
  dockOnboardingStore.setReady("tour", true);
}

describe("dock onboarding readiness", () => {
  beforeEach(() => {
    vi.stubGlobal("localStorage", memoryStorage());
    vi.stubGlobal("window", { setTimeout: () => 0, clearTimeout: () => {} });
    resetOnboardingSingleton();
  });

  afterEach(() => {
    resetOnboardingSingleton();
    vi.unstubAllGlobals();
  });

  it("holds the settings tip until a file is open, so a fresh profile meets the toggle tip before it", () => {
    let fileOpen = false;
    const anchors = new Map<string, FakeTarget>([
      ["[data-dock-viewer]", fakeViewer()],
      [tipAnchorSelector("compose"), fakeTarget({ disabled: () => !fileOpen })],
      [tipAnchorSelector("config"), fakeTarget()],
    ]);
    const rootRef = { current: { querySelector: (selector: string) => anchors.get(selector) ?? null } as unknown as HTMLElement };
    hooks.render(DockOnboarding, { rootRef, layout: DEFAULT_DOCK_LAYOUT, hasFile: false });
    dockOnboardingStore.show();
    expect(dockOnboardingStore.get().card).toBe("tour");
    fileOpen = true;
    hooks.render(DockOnboarding, { rootRef, layout: DEFAULT_DOCK_LAYOUT, hasFile: true });
    dockOnboardingStore.show();
    expect(dockOnboardingStore.get().card).toBe("toggle");
  });
});

describe("closing the drag tour", () => {
  beforeEach(() => {
    vi.stubGlobal("localStorage", memoryStorage());
    vi.stubGlobal("window", { innerWidth: 1280, innerHeight: 800, addEventListener() {}, removeEventListener() {} });
    vi.stubGlobal("document", { activeElement: null, body: {} });
    vi.stubGlobal("requestAnimationFrame", (run: (time: number) => void) => {
      run(0);
      return 1;
    });
    vi.stubGlobal("cancelAnimationFrame", () => {});
    vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
    vi.stubGlobal("MutationObserver", class { observe() {} disconnect() {} });
    resetOnboardingSingleton();
  });

  afterEach(() => {
    resetOnboardingSingleton();
    vi.unstubAllGlobals();
  });

  function railTriggerSelector(): string {
    const asked: string[] = [];
    rightRailToolsTrigger({
      querySelector: (selector: string) => {
        asked.push(selector);
        return null;
      },
    } as unknown as ParentNode);
    return asked[0];
  }

  function closeTour(tools: FakeTarget[], trigger: FakeTarget): void {
    const viewer = fakeViewer();
    const triggerSelector = railTriggerSelector();
    const root = {
      querySelector: (selector: string) =>
        selector === "[data-dock-viewer]" ? viewer : selector === triggerSelector ? trigger : selector.endsWith("[data-tool-id]") ? (tools[0] ?? null) : null,
      querySelectorAll: (selector: string) => (selector.endsWith("[data-tool-id]") ? tools : []),
    };
    const rootRef = { current: root as unknown as HTMLElement };
    expect(hooks.render(DockOnboardingPopup, { rootRef, step: 0 })).toBeNull();
    const card = hooks.render(DockOnboardingPopup, { rootRef, step: 0 }) as ReactElement<DockOnboardingCardProps>;
    expect(card.type).toBe(DockOnboardingCard);
    (card.props.cardRef as { current: unknown }).current = { contains: () => true };
    card.props.onClose();
  }

  it("moves the focus to the first tool the right rail renders, not to a hidden one", () => {
    const hidden = fakeTarget({ rendered: false });
    const shown = fakeTarget();
    const trigger = fakeTarget();
    closeTour([hidden, shown], trigger);
    expect(hidden.calls).toEqual([]);
    expect(shown.calls).toEqual(["focus", "scroll"]);
    expect(trigger.calls).toEqual([]);
  });

  it("falls back to the All tools trigger when the right rail renders no tool", () => {
    const hidden = fakeTarget({ rendered: false });
    const trigger = fakeTarget();
    closeTour([hidden], trigger);
    expect(hidden.calls).toEqual([]);
    expect(trigger.calls).toEqual(["focus", "scroll"]);
  });
});
