import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { hooks } from "../../../test/hookRuntime";
import { findElement } from "../../../test/elementTree";
import { regionStore } from "../../../utils/regionStore";
import { appModeStore } from "../../../hooks/useAppMode";

vi.mock("react", async (importOriginal) => {
  const { reactWithFakeHooks } = await import("../../../test/hookRuntime");
  return reactWithFakeHooks(await importOriginal());
});

vi.mock("react-dom", () => ({ createPortal: (children: ReactNode) => children }));

import RegionToolbar from "../RegionToolbar";

interface ElementProps {
  id?: string;
  title?: string;
  role?: string;
  className?: string;
  ref?: { current: unknown };
  "aria-label"?: string;
  "aria-pressed"?: boolean;
  "aria-expanded"?: boolean;
  "aria-controls"?: string;
  "aria-describedby"?: string;
  children?: ReactNode;
  onClick?: () => void;
}

class FakeNode {
  constructor(readonly parent: FakeNode | null = null) {}

  contains(other: unknown): boolean {
    let node = other;
    while (node instanceof FakeNode) {
      if (node === this) return true;
      node = node.parent;
    }
    return false;
  }
}

type PointerListener = (event: { target: unknown }) => void;

const documentListeners = new Map<string, Set<PointerListener>>();

function listenerKey(type: string, options?: boolean | AddEventListenerOptions): string {
  const capture = typeof options === "boolean" ? options : options?.capture === true;
  return `${type}:${capture ? "capture" : "bubble"}`;
}

function fire(type: string, phase: "capture" | "bubble", target: unknown): void {
  for (const listener of [...(documentListeners.get(`${type}:${phase}`) ?? [])]) listener({ target });
}

function attach(props: ElementProps | null): FakeNode {
  if (!props?.ref) throw new Error("Missing ref");
  const node = new FakeNode();
  props.ref.current = node;
  return node;
}

const HELP_CSS = readFileSync(fileURLToPath(new URL("../region-tool-help.css", import.meta.url)), "utf8");

function find(node: ReactNode, predicate: (props: ElementProps) => boolean): ElementProps | null {
  return findElement<ElementProps>(node, (element) => predicate(element.props))?.props ?? null;
}

function render(): ReactNode {
  return hooks.render((RegionToolbar as unknown as { type: () => ReactNode }).type, undefined) as ReactNode;
}

function click(title: string): void {
  const button = find(render(), (props) => props.title === title);
  if (!button?.onClick) throw new Error(`Missing button ${title}`);
  button.onClick();
}

function hint(): ElementProps | null {
  return find(render(), (props) => props.role === "note" && props["aria-label"] === "Line cut help");
}

beforeEach(() => {
  hooks.mount();
  regionStore.setTool("none");
  regionStore.clearDraft();
  appModeStore.set("workspace");
  const values = new Map<string, string>();
  vi.stubGlobal("sessionStorage", {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => values.set(key, value),
  });
  documentListeners.clear();
  vi.stubGlobal("Node", FakeNode);
  vi.stubGlobal("document", {
    body: {},
    addEventListener: (type: string, listener: PointerListener, options?: boolean | AddEventListenerOptions) => {
      const key = listenerKey(type, options);
      documentListeners.set(key, (documentListeners.get(key) ?? new Set()).add(listener));
    },
    removeEventListener: (type: string, listener: PointerListener, options?: boolean | AddEventListenerOptions) => {
      documentListeners.get(listenerKey(type, options))?.delete(listener);
    },
  });
});

afterEach(() => {
  hooks.mount();
  regionStore.setTool("none");
  regionStore.clearDraft();
  appModeStore.set("workspace");
  vi.unstubAllGlobals();
});

describe("RegionToolbar contextual line guidance", () => {
  it("does not show guidance at startup, for other tools, or for an already active line", () => {
    expect(hint()).toBeNull();
    click("Draw circle");
    expect(hint()).toBeNull();
    regionStore.setTool("line");
    hooks.mount();
    expect(hint()).toBeNull();
  });

  it("shows guidance only after activating line cut and preserves draft clearing", () => {
    regionStore.setDraft("frame", { shape: "line", x1: 0, y1: 0, x2: 10, y2: 10 });
    click("Draw line cut");
    expect(regionStore.getTool()).toBe("line");
    expect(regionStore.getDraftFor("frame")).toBeNull();
    expect(hint()).not.toBeNull();
    expect(find(render(), (props) => props.title === "Draw line cut")?.["aria-pressed"]).toBe(true);
    const html = renderToStaticMarkup(render());
    expect(html).toContain("Drag from start to end on the image.");
    expect(html).toContain("Image &gt; Profiles shows the line cut and length");
    expect(html).toContain('title="Draw line cut"');
  });

  it("dismisses without changing the tool and offers an explicit way to reopen", () => {
    click("Draw line cut");
    click("Dismiss line cut help");
    expect(regionStore.getTool()).toBe("line");
    expect(hint()).toBeNull();
    click("Line cut help");
    expect(hint()).not.toBeNull();
  });

  it("shows automatically only once per session, including after the toolbar remounts", () => {
    click("Draw line cut");
    expect(hint()).not.toBeNull();
    click("Draw line cut");
    expect(regionStore.getTool()).toBe("none");
    expect(hint()).toBeNull();
    hooks.mount();
    click("Draw line cut");
    expect(hint()).toBeNull();
    click("Line cut help");
    expect(hint()).not.toBeNull();
  });

  it("hides guidance when another tool is selected or drawing completes", () => {
    click("Draw line cut");
    click("Draw circle");
    expect(regionStore.getTool()).toBe("circle");
    expect(hint()).toBeNull();
    click("Draw line cut");
    click("Line cut help");
    expect(hint()).not.toBeNull();
    regionStore.setTool("select");
    expect(hint()).toBeNull();
  });

  it("closes when leaving the workspace and does not reopen on return", () => {
    click("Draw line cut");
    expect(hint()).not.toBeNull();
    appModeStore.set("archive");
    expect(hint()).toBeNull();
    appModeStore.set("workspace");
    expect(hint()).toBeNull();
  });
});

describe("RegionToolbar line help dismissal", () => {
  it("closes on a capture-phase pointerdown outside the card and keeps the line tool", () => {
    click("Draw line cut");
    expect(hint()).not.toBeNull();
    fire("pointerdown", "capture", new FakeNode());
    expect(hint()).toBeNull();
    expect(regionStore.getTool()).toBe("line");
  });

  it("stays open for a pointerdown on the card, inside it, on the line button or on the help trigger", () => {
    click("Draw line cut");
    const tree = render();
    const card = attach(find(tree, (props) => props.role === "note"));
    const line = attach(find(tree, (props) => props.title === "Draw line cut"));
    const trigger = attach(find(tree, (props) => props.title === "Line cut help"));
    for (const target of [card, new FakeNode(card), line, trigger]) {
      fire("pointerdown", "capture", target);
      expect(hint()).not.toBeNull();
    }
  });

  it("removes its capture-phase listener once the help closes", () => {
    click("Draw line cut");
    expect(hint()).not.toBeNull();
    expect(documentListeners.get("pointerdown:capture")?.size).toBe(1);
    click("Dismiss line cut help");
    expect(hint()).toBeNull();
    expect(documentListeners.get("pointerdown:capture")?.size ?? 0).toBe(0);
    expect(documentListeners.get("pointerdown:bubble")?.size ?? 0).toBe(0);
  });
});

describe("RegionToolbar line help semantics", () => {
  it("points the trigger at the note and describes the line button with the paragraph", () => {
    click("Draw line cut");
    const tree = render();
    const note = find(tree, (props) => props.role === "note");
    const trigger = find(tree, (props) => props.title === "Line cut help");
    const line = find(tree, (props) => props.title === "Draw line cut");
    expect(note?.id).toBeTruthy();
    expect(trigger?.["aria-controls"]).toBe(note?.id);
    expect(trigger?.["aria-expanded"]).toBe(true);
    expect(line?.["aria-describedby"]).toBeTruthy();
    expect(line?.["aria-describedby"]).not.toBe(note?.id);
    const body = find(tree, (props) => props.id === line?.["aria-describedby"]);
    expect(String(body?.children)).toContain("Drag from start to end on the image.");
  });

  it("keeps the help trigger's slot while another tool is active, hidden and without popup state", () => {
    click("Draw circle");
    const idle = find(render(), (props) => props.title === "Line cut help");
    expect(idle).not.toBeNull();
    expect(idle?.className).toContain("ab-region-help-trigger-idle");
    expect(idle?.["aria-controls"]).toBeUndefined();
    expect(idle?.["aria-expanded"]).toBeUndefined();
    click("Draw line cut");
    const active = find(render(), (props) => props.title === "Line cut help");
    expect(active?.className).not.toContain("ab-region-help-trigger-idle");
    expect(active?.["aria-expanded"]).toBe(true);
  });

  it("hides the idle trigger with visibility so it keeps its width", () => {
    expect(HELP_CSS).toMatch(/\.ab-region-help-trigger-idle\s*\{[^}]*visibility:\s*hidden/);
  });
});
