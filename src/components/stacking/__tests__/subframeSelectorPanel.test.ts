import { describe, expect, it, vi } from "vitest";
import { createElement, type ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { SubframeStatusButton, ThSort } from "../SubframeSelectorPanel";

type ClickProps = { onClick: (e: { stopPropagation: () => void }) => void };

function sortHeader(col: "file_name" | "weight", current: "file_name" | "weight", asc: boolean, onClick = vi.fn()): string {
  const th = createElement(ThSort, { label: col === "weight" ? "Weight" : "File", col, current, asc, onClick });
  return renderToStaticMarkup(createElement("table", null, createElement("thead", null, createElement("tr", null, th))));
}

function buttonIn(node: unknown): ReactElement<ClickProps> | null {
  if (!node || typeof node !== "object" || !("type" in node)) return null;
  const el = node as ReactElement<{ children?: unknown }>;
  if (el.type === "button") return el as unknown as ReactElement<ClickProps>;
  for (const child of [el.props.children].flat()) {
    const found = buttonIn(child);
    if (found) return found;
  }
  return null;
}

function findButton(node: unknown): ReactElement<ClickProps> {
  const button = buttonIn(node);
  if (!button) throw new Error("no button rendered");
  return button;
}

describe("Subframe status cell", () => {
  it("is a toggle button that announces an accepted frame", () => {
    const html = renderToStaticMarkup(createElement(SubframeStatusButton, { name: "light_001.fits", accepted: true, onToggle: () => {} }));
    expect(html).toMatch(/^<button type="button"/);
    expect(html).toContain('aria-pressed="true"');
    expect(html).toContain('aria-label="Frame light_001.fits: accepted, press to reject"');
  });

  it("announces a rejected frame and the action that accepts it", () => {
    const html = renderToStaticMarkup(createElement(SubframeStatusButton, { name: "light_002.fits", accepted: false, onToggle: () => {} }));
    expect(html).toContain('aria-pressed="false"');
    expect(html).toContain('aria-label="Frame light_002.fits: rejected, press to accept"');
  });

  it("toggles once per activation without also firing the row click", () => {
    const onToggle = vi.fn();
    const stopPropagation = vi.fn();
    const button = findButton(SubframeStatusButton({ name: "light_001.fits", accepted: true, onToggle }));
    button.props.onClick({ stopPropagation });
    expect(onToggle).toHaveBeenCalledTimes(1);
    expect(stopPropagation).toHaveBeenCalledTimes(1);
  });
});

describe("Subframe sort headers", () => {
  it("mark the sorted column with aria-sort and wrap the label in a button", () => {
    expect(sortHeader("weight", "weight", false)).toMatch(/<th aria-sort="descending"[^>]*><button type="button"[^>]*>Weight/);
    expect(sortHeader("weight", "weight", true)).toContain('aria-sort="ascending"');
  });

  it("mark the other sortable columns as unsorted", () => {
    expect(sortHeader("file_name", "weight", false)).toMatch(/<th aria-sort="none"[^>]*><button type="button"[^>]*>File<\/button><\/th>/);
  });

  it("keep the direction arrow out of the accessible name", () => {
    expect(sortHeader("weight", "weight", false)).toMatch(/<span aria-hidden="true"[^>]*>▼<\/span>/);
  });

  it("sort by the column when the button is activated", () => {
    const onClick = vi.fn();
    const button = findButton(ThSort({ label: "File", col: "file_name", current: "weight", asc: false, onClick }));
    button.props.onClick({ stopPropagation: () => {} });
    expect(onClick).toHaveBeenCalledWith("file_name");
  });
});
