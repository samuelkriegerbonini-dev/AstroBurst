import { describe, expect, it, vi } from "vitest";
import { createElement, isValidElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import StaleResultNote, { ReopenedProductNote } from "../StaleResultNote";

type Props = Record<string, unknown> & { children?: ReactNode };

function elements(node: ReactNode): Props[] {
  if (Array.isArray(node)) return node.flatMap(elements);
  if (!isValidElement<Props>(node)) return [];
  return [node.props, ...elements(node.props.children)];
}

describe("StaleResultNote", () => {
  it("announces the stale result as a status with the fixed text", () => {
    const html = renderToStaticMarkup(createElement(StaleResultNote));
    expect(html).toContain('role="status"');
    expect(html).toContain("data-stale-result");
    expect(html).toContain(">Stale: inputs changed since this result. Run again to update.<");
  });
});

describe("ReopenedProductNote", () => {
  it("names the product on screen and says its parameters and manifest are not shown", () => {
    const html = renderToStaticMarkup(createElement(ReopenedProductNote, { label: "Quick slope (DN/s)", onShowCube: () => {} }));
    expect(html).toContain('role="status"');
    expect(html).toContain("data-reopened-product");
    expect(html).toContain("Quick slope (DN/s) on screen is from an earlier run. Its parameters and manifest are not shown here; run again to replace it.");
    expect(html).toContain(">Show cube</button>");
  });

  it("offers Show cube only with an action and calls it", () => {
    expect(renderToStaticMarkup(createElement(ReopenedProductNote, { label: "Moment m0 · 35 ch" }))).not.toContain("<button");
    const showCube = vi.fn();
    const button = elements(ReopenedProductNote({ label: "Moment m0 · 35 ch", onShowCube: showCube })).find((p) => p.children === "Show cube");
    (button?.onClick as () => void)();
    expect(showCube).toHaveBeenCalledTimes(1);
  });

  it("moves focus to the given target before Show cube removes the note and its button", () => {
    const steps: string[] = [];
    const target = { current: { focus: () => steps.push("focus") } as unknown as HTMLElement };
    const note = ReopenedProductNote({ label: "Moment m0 · 35 ch", onShowCube: () => steps.push("show cube"), focusAfterShowCube: target });
    const button = elements(note).find((p) => p.children === "Show cube");
    (button?.onClick as () => void)();
    expect(steps).toEqual(["focus", "show cube"]);
  });
});
