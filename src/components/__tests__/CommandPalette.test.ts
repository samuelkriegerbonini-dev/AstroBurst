import { describe, expect, it } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import CommandPalette from "../CommandPalette";

function renderPalette(open: boolean): string {
  return renderToStaticMarkup(
    createElement(CommandPalette, {
      open,
      onClose: () => {},
      actions: [
        { id: "open-files", label: "Open FITS Files...", run: () => {} },
        { id: "tool-config", label: "Open Settings Panel", keywords: ["Config"], run: () => {} },
      ],
      files: [{ id: "f1", name: "656nmos.fits", filter: "F656N" }],
      selectedId: null,
      onSelectFile: () => {},
    }),
  );
}

function attr(html: string, pattern: RegExp): string {
  const match = pattern.exec(html);
  if (!match) throw new Error(`pattern ${pattern} not found`);
  return match[1];
}

describe("CommandPalette accessibility", () => {
  it("renders nothing while closed", () => {
    expect(renderPalette(false)).toBe("");
  });

  it("is a labelled modal dialog", () => {
    const html = renderPalette(true);
    expect(html).toMatch(/role="dialog"[^>]*aria-modal="true"/);
    expect(html).toMatch(/role="dialog"[^>]*aria-label="Search everywhere"/);
  });

  it("wires the input as a combobox that controls the result list", () => {
    const html = renderPalette(true);
    const listId = attr(html, /role="listbox"[^>]*id="([^"]+)"/);
    expect(html).toMatch(new RegExp(`role="combobox"[^>]*aria-controls="${listId}"`));
    expect(html).toMatch(/role="combobox"[^>]*aria-expanded="true"/);
  });

  it("points the active descendant at the selected option", () => {
    const html = renderPalette(true);
    const active = attr(html, /aria-activedescendant="([^"]+)"/);
    expect(html).toMatch(new RegExp(`id="${active}"[^>]*role="option"[^>]*aria-selected="true"`));
  });

  it("marks every row as an option and only one as selected", () => {
    const html = renderPalette(true);
    expect(html.match(/role="option"/g)).toHaveLength(3);
    expect(html.match(/aria-selected="true"/g)).toHaveLength(1);
    expect(html.match(/aria-selected="false"/g)).toHaveLength(2);
  });

  it("keeps the rows out of the tab order so Tab stays in the input", () => {
    const html = renderPalette(true);
    expect(html.match(/role="option"[^>]*tabindex="-1"/g)).toHaveLength(3);
  });
});
