import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactNode } from "react";
import { hooks } from "../../test/hookRuntime";
import { dockStore } from "../../hooks/useDockLayout";
import { toolBrowserStore, useToolBrowserOpen } from "../dock/toolBrowserStore";
import DockShell from "../dock/DockShell";
import Stripe from "../dock/Stripe";
import ToolBrowser from "../dock/ToolBrowser";

vi.mock("react", async (original) => {
  const { reactWithFakeHooks } = await import("../../test/hookRuntime");
  return reactWithFakeHooks(await original<Record<string, unknown>>());
});

vi.mock("../../context/PreviewContext", async (original) => ({
  ...(await original<Record<string, unknown>>()),
  useFileContext: () => ({ file: null }),
  useCubeContext: () => ({ isCube: false, isSpectrum: false, ramp: null, cubeFileKey: null }),
}));

type Node = { type: unknown; props: Record<string, unknown> };

function elements(node: ReactNode): Node[] {
  if (Array.isArray(node)) return node.flatMap(elements);
  if (!isValidElement<Record<string, unknown>>(node)) return [];
  return [{ type: node.type, props: node.props }, ...elements(node.props.children as ReactNode)];
}

function shell(): Node[] {
  return elements(hooks.render(DockShell, { tools: {}, children: null }) as ReactNode);
}

beforeEach(() => {
  hooks.mount();
  toolBrowserStore.close();
  dockStore.dispatch({ type: "reset" });
});

afterEach(() => {
  hooks.mount();
  toolBrowserStore.close();
  dockStore.dispatch({ type: "reset" });
});

describe("toolBrowserStore", () => {
  it("starts closed, opens and closes, and notifies only on a change", () => {
    const listener = vi.fn();
    const unsubscribe = toolBrowserStore.subscribe(listener);
    expect(toolBrowserStore.get()).toBe(false);
    toolBrowserStore.open();
    toolBrowserStore.open();
    expect(toolBrowserStore.get()).toBe(true);
    expect(listener).toHaveBeenCalledTimes(1);
    toolBrowserStore.close();
    toolBrowserStore.close();
    expect(toolBrowserStore.get()).toBe(false);
    expect(listener).toHaveBeenCalledTimes(2);
    unsubscribe();
    toolBrowserStore.open();
    expect(listener).toHaveBeenCalledTimes(2);
  });

  it("reads the open state through useToolBrowserOpen", () => {
    expect(hooks.render(useToolBrowserOpen, undefined)).toBe(false);
    toolBrowserStore.open();
    expect(hooks.render(useToolBrowserOpen, undefined)).toBe(true);
  });
});

describe("DockShell and the All tools dialog", () => {
  it("opens the dialog from the rail trigger through the store and closes it through the store", () => {
    const first = shell();
    expect(first.some((node) => node.type === ToolBrowser)).toBe(false);
    const right = first.find((node) => node.type === Stripe && node.props.side === "right")!;
    (right.props.onBrowseTools as () => void)();
    expect(toolBrowserStore.get()).toBe(true);
    const browser = shell().find((node) => node.type === ToolBrowser)!;
    (browser.props.onClose as () => void)();
    expect(toolBrowserStore.get()).toBe(false);
    expect(shell().some((node) => node.type === ToolBrowser)).toBe(false);
  });

  it("shows the dialog when another surface opens the store", () => {
    shell();
    toolBrowserStore.open();
    expect(shell().some((node) => node.type === ToolBrowser)).toBe(true);
  });
});
