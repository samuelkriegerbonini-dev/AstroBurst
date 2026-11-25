import { afterAll, describe, expect, it, vi } from "vitest";
import { createElement, isValidElement, type ReactElement, type ReactNode } from "react";
import { elementsOf, findElement, textOf } from "../../../test/elementTree";

vi.hoisted(() => {
  vi.stubGlobal("document", {
    createElement: () => ({ nodeType: 1, dataset: {}, className: "", hidden: false, inert: false, parentElement: null, remove: () => {}, querySelectorAll: () => [] }),
  });
});

vi.mock("react", async (importOriginal) => ({
  ...(await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()),
  useInsertionEffect: (create: () => void) => create(),
}));

import { hooks } from "../../../test/hookRuntime";
import ToolMount, { type ToolMountProps } from "../../dock/ToolMount";
import PanelErrorBoundary, { PANEL_FAILED_PREFIX, RELOAD_PANEL_LABEL } from "../PanelErrorBoundary";

afterAll(() => {
  vi.unstubAllGlobals();
});

type Props = Record<string, unknown> & { children?: ReactNode };

function boundary(children: ReactNode, name = "Photometry") {
  const instance = new PanelErrorBoundary({ name, children });
  instance.setState = ((update: Partial<typeof instance.state>) => {
    instance.state = { ...instance.state, ...update };
  }) as typeof instance.setState;
  return instance;
}

function fail(instance: PanelErrorBoundary, error: Error): void {
  instance.state = { ...instance.state, ...PanelErrorBoundary.getDerivedStateFromError(error) };
}

describe("PanelErrorBoundary", () => {
  it("shows the error inside the panel with a way to reload it, instead of replacing the window", () => {
    const instance = boundary(createElement("i", { "data-panel-body": "" }));
    const errorLog = vi.spyOn(console, "error").mockImplementation(() => {});
    fail(instance, new Error("Cannot read properties of null (reading 'zero_point')"));
    instance.componentDidCatch(new Error("x"), { componentStack: "" });
    errorLog.mockRestore();

    const tree = instance.render();
    const alert = findElement<Props>(tree, (el) => el.props.role === "alert");
    expect(alert, "alert in the panel").toBeDefined();
    expect(textOf(alert)).toContain(`${PANEL_FAILED_PREFIX}Cannot read properties of null (reading 'zero_point')`);
    expect(textOf(alert)).toContain("Photometry");
    const reload = findElement<Props>(tree, (el) => el.type === "button");
    expect(textOf(reload)).toBe(RELOAD_PANEL_LABEL);
    expect(findElement<Props>(tree, (el) => el.props["data-panel-body"] !== undefined)).toBeUndefined();
  });

  it("mounts the panel again under a new key when Reload panel is pressed", () => {
    const instance = boundary(createElement("i", { "data-panel-body": "" }));
    const before = instance.render() as ReactElement;
    fail(instance, new Error("boom"));
    const reload = findElement<Props>(instance.render(), (el) => el.type === "button");
    (reload?.props.onClick as () => void)();
    const after = instance.render() as ReactElement;

    expect(instance.state.error).toBeNull();
    expect(findElement<Props>(after, (el) => el.props["data-panel-body"] !== undefined)).toBeDefined();
    expect(after.key).not.toBe(before.key);
  });
});

describe("ToolMount", () => {
  function Throws(): never {
    throw new Error("panel render failed");
  }

  it("puts each tool panel inside its own error boundary", () => {
    const props: ToolMountProps = {
      tool: "photometry",
      content: createElement(Throws),
      containers: { current: { "left-top": null, "left-bottom": null, "right-top": null, "right-bottom": null } },
      anchor: "right-top",
      activeTool: "photometry",
      shownTool: "photometry",
      anchorMounted: true,
      hasFile: true,
      fileKey: "k",
      groupActive: false,
    };
    const inner = (ToolMount as unknown as { type: (p: ToolMountProps) => unknown }).type;
    hooks.mount();
    hooks.render(inner, props);
    const portal = hooks.render(inner, props) as { children?: ReactNode } | null;
    expect(portal, "the tool is mounted").not.toBeNull();
    const nodes = elementsOf<Props>(portal?.children);
    const guard = nodes.findIndex((el) => el.type === PanelErrorBoundary);
    const content = nodes.findIndex((el) => el.type === Throws);
    expect(guard, "error boundary around the tool").toBeGreaterThanOrEqual(0);
    expect(content).toBeGreaterThan(guard);
    expect(isValidElement(nodes[guard]) && (nodes[guard].props as Props).name).toBe("Photometry");
    hooks.mount();
  });
});
