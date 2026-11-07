import { afterEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import AppFooter from "../AppFooter";
import type { AppMode } from "../../hooks/useAppMode";

const PROPS = { mode: "workspace" as AppMode, onModeChange: () => {}, onOpenPalette: () => {}, onOpenShortcuts: () => {} };

function footer(platform: string): string {
  vi.stubGlobal("navigator", { platform });
  return renderToStaticMarkup(createElement(AppFooter, PROPS));
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("AppFooter in the empty workspace", () => {
  it("shows Search, the shortcut sheet key and the mode switch with no mode slot", () => {
    const html = footer("Win32");
    expect(html).toMatch(/>Search<kbd class="ab-cmdp-kbd">Ctrl\+K<\/kbd>/);
    expect(html).toContain('<kbd class="ab-cmdp-kbd">?</kbd>');
    expect(html).toContain('data-testid="mode-switch"');
    expect(html).not.toMatch(/class="w-px h-3"/);
  });

  it("shows the Command key for Search on macOS, as the shortcut sheet does", () => {
    const html = footer("MacIntel");
    expect(html).toContain('<kbd class="ab-cmdp-kbd">⌘K</kbd>');
    expect(html).toContain('title="Search everywhere (⌘K or double Shift)"');
    expect(html).not.toContain("Ctrl+K");
  });
});
