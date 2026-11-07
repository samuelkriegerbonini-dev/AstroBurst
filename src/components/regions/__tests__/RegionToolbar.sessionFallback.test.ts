import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactNode } from "react";
import { hooks } from "../../../test/hookRuntime";

vi.mock("react", async (importOriginal) => {
  const { reactWithFakeHooks } = await import("../../../test/hookRuntime");
  return reactWithFakeHooks(await importOriginal());
});

vi.mock("react-dom", () => ({ createPortal: (children: ReactNode) => children }));

interface ElementProps {
  title?: string;
  role?: string;
  children?: ReactNode;
  onClick?: () => void;
}

function find(node: ReactNode, predicate: (props: ElementProps) => boolean): ElementProps | null {
  if (Array.isArray(node)) {
    for (const child of node) {
      const result = find(child, predicate);
      if (result) return result;
    }
    return null;
  }
  if (!isValidElement<ElementProps>(node)) return null;
  if (predicate(node.props)) return node.props;
  return find(node.props.children, predicate);
}

async function freshToolbar() {
  vi.resetModules();
  const { regionStore } = await import("../../../utils/regionStore");
  const { appModeStore } = await import("../../../hooks/useAppMode");
  const { default: RegionToolbar } = await import("../RegionToolbar");
  hooks.mount();
  regionStore.setTool("none");
  appModeStore.set("workspace");
  const render = () => hooks.render((RegionToolbar as unknown as { type: () => ReactNode }).type, undefined) as ReactNode;
  const click = (title: string) => {
    const button = find(render(), (props) => props.title === title);
    if (!button?.onClick) throw new Error(`Missing button ${title}`);
    button.onClick();
  };
  const helpShown = () => find(render(), (props) => props.role === "note") !== null;
  return { regionStore, click, helpShown };
}

beforeEach(() => {
  const blocked = () => {
    throw new Error("SecurityError: sessionStorage is not available");
  };
  vi.stubGlobal("sessionStorage", { getItem: blocked, setItem: blocked });
  vi.stubGlobal("document", { body: {}, addEventListener: () => {}, removeEventListener: () => {} });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("RegionToolbar line help without sessionStorage", () => {
  it("shows the help once per session without sessionStorage", async () => {
    const { click, helpShown, regionStore } = await freshToolbar();
    click("Draw line cut");
    expect(helpShown()).toBe(true);
    click("Draw line cut");
    expect(regionStore.getTool()).toBe("none");
    click("Draw line cut");
    expect(regionStore.getTool()).toBe("line");
    expect(helpShown()).toBe(false);
    click("Line cut help");
    expect(helpShown()).toBe(true);
  });

  it("keeps the once-per-session flag across a remount of the toolbar", async () => {
    const { click, helpShown } = await freshToolbar();
    click("Draw line cut");
    expect(helpShown()).toBe(true);
    click("Draw line cut");
    hooks.mount();
    click("Draw line cut");
    expect(helpShown()).toBe(false);
  });
});
