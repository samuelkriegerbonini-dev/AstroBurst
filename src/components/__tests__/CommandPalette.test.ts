import { afterEach, describe, expect, it, vi } from "vitest";
import { createElement, isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";

vi.mock("react", async (importOriginal) => (await import("../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));

import { hooks } from "../../test/hookRuntime";
import CommandPalette, { HIDDEN_FILE_HINT, type PaletteFile } from "../CommandPalette";

type AnyElement = ReactElement<Record<string, unknown> & { children?: ReactNode }>;

function elements(node: ReactNode): AnyElement[] {
  if (Array.isArray(node)) return node.flatMap(elements);
  if (!isValidElement(node)) return [];
  const el = node as AnyElement;
  return [el, ...elements(el.props.children)];
}

afterEach(() => {
  hooks.mount();
  vi.unstubAllGlobals();
});

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

describe("CommandPalette running an entry", () => {
  it("closes before it runs the chosen entry, so an entry can reopen the palette to ask for a confirmation", () => {
    vi.stubGlobal("window", { addEventListener: () => {}, removeEventListener: () => {} });
    vi.stubGlobal("document", { activeElement: null, querySelector: () => null });
    vi.stubGlobal("HTMLElement", class {});
    vi.stubGlobal("requestAnimationFrame", () => 1);
    vi.stubGlobal("cancelAnimationFrame", () => {});
    const calls: string[] = [];
    hooks.mount();
    const tree = hooks.render(CommandPalette, {
      open: true,
      onClose: () => calls.push("close"),
      actions: [{ id: "new-batch", label: "New Batch (discard processed files)", run: () => calls.push("run") }],
      files: [],
      selectedId: null,
      onSelectFile: () => {},
    }) as ReactNode;
    const option = elements(tree).find((el) => el.props.role === "option");
    (option?.props.onClick as () => void)();
    expect(calls).toEqual(["close", "run"]);
  });
});

describe("CommandPalette returning focus", () => {
  const TOOLS_TRIGGER = '[data-dock-strip="right"] [data-tools-trigger]';

  class FakeElement {}

  interface Page {
    activeElement: unknown;
    body?: unknown;
  }

  function focusable(page: Page, name: string) {
    let rendered = true;
    const element = Object.assign(new FakeElement(), {
      name,
      isConnected: true,
      getClientRects: () => (rendered ? [{}] : []),
      closest: () => ({}),
      scrollIntoView: vi.fn(),
      focus: vi.fn(() => {
        if (element.isConnected && rendered) page.activeElement = element;
      }),
      hide() {
        rendered = false;
      },
      remove() {
        rendered = false;
        element.isConnected = false;
      },
    });
    return element;
  }

  function stubPage() {
    const page: Page = { activeElement: null };
    const body = focusable(page, "body");
    const trigger = focusable(page, "All tools trigger");
    page.body = body;
    page.activeElement = body;
    vi.stubGlobal("window", { addEventListener: () => {}, removeEventListener: () => {} });
    vi.stubGlobal("document", Object.assign(page, { querySelector: (selector: string) => (selector === TOOLS_TRIGGER ? trigger : null) }));
    vi.stubGlobal("HTMLElement", FakeElement);
    vi.stubGlobal("requestAnimationFrame", () => 1);
    vi.stubGlobal("cancelAnimationFrame", () => {});
    return { page, trigger };
  }

  const palette = (open: boolean) => ({
    open,
    onClose: () => {},
    actions: [{ id: "reset-layout", label: "Reset layout", run: () => {} }],
    files: [],
    selectedId: null,
    onSelectFile: () => {},
  });

  function openThenClose(page: Page, whileOpen: () => void = () => {}): void {
    hooks.mount();
    hooks.render(CommandPalette, palette(true));
    page.activeElement = page.body;
    whileOpen();
    hooks.render(CommandPalette, palette(false));
  }

  const focusedName = (page: Page) => (page.activeElement as { name: string }).name;

  it("gives focus back to the element that had it before the palette opened while that element is still shown", () => {
    const { page, trigger } = stubPage();
    page.activeElement = focusable(page, "Stacking rail button");
    openThenClose(page);
    expect(focusedName(page)).toBe("Stacking rail button");
    expect(trigger.focus).not.toHaveBeenCalled();
  });

  it("leaves focus on the page body when nothing had focus before the palette opened", () => {
    const { page, trigger } = stubPage();
    openThenClose(page);
    expect(focusedName(page)).toBe("body");
    expect(trigger.focus).not.toHaveBeenCalled();
  });

  it.each([
    ["hid it", false],
    ["moved it to another group", true],
  ])("falls back to the right rail's All tools trigger when Reset layout %s", (_change, moved) => {
    const { page, trigger } = stubPage();
    const stacking = focusable(page, "Stacking rail button");
    page.activeElement = stacking;
    openThenClose(page, () => (moved ? stacking.remove() : stacking.hide()));
    expect(focusedName(page)).toBe("All tools trigger");
    expect(trigger.focus).toHaveBeenCalledWith({ preventScroll: true });
    expect(trigger.scrollIntoView).toHaveBeenCalledWith({ block: "nearest" });
  });
});

describe("CommandPalette files", () => {
  const NIRCAM_NAME = "jw02739001001_02105_00001_nrca1_cal.fits";

  function stubBrowser(): void {
    vi.stubGlobal("window", { addEventListener: () => {}, removeEventListener: () => {} });
    vi.stubGlobal("document", { activeElement: null, querySelector: () => null });
    vi.stubGlobal("HTMLElement", class {});
    vi.stubGlobal("requestAnimationFrame", () => 1);
    vi.stubGlobal("cancelAnimationFrame", () => {});
  }

  function paletteAfterTyping(files: PaletteFile[], query: string): AnyElement[] {
    stubBrowser();
    hooks.mount();
    const props = { open: true, onClose: () => {}, actions: [], files, selectedId: null, onSelectFile: () => {} };
    const render = () => elements(hooks.render(CommandPalette, props) as ReactNode);
    const combobox = render().find((el) => el.props.role === "combobox")!;
    (combobox.props.onChange as (e: { target: { value: string } }) => void)({ target: { value: query } });
    return render();
  }

  const options = (tree: AnyElement[]) => tree.filter((el) => el.props.role === "option");
  const optionLabels = (tree: AnyElement[]) =>
    tree.filter((el) => el.props.className === "ab-cmdp-item-label").map((el) => String((el.props.children as unknown[])[0]));
  const markup = (tree: AnyElement[]) => renderToStaticMarkup(tree[0]);

  it("finds a file by the text the Files search matches", () => {
    const withSearchText: PaletteFile[] = [{ id: "f1", name: NIRCAM_NAME, filter: "F200W", searchText: `${NIRCAM_NAME}\nf200w\nnircam` }];
    const byInstrument = paletteAfterTyping(withSearchText, "nircam");
    expect(options(byInstrument)).toHaveLength(1);
    expect(optionLabels(byInstrument)).toEqual([NIRCAM_NAME]);

    const noMatch = paletteAfterTyping(withSearchText, "wfc3");
    expect(options(noMatch)).toHaveLength(0);
    expect(markup(noMatch)).toContain("No matches for");

    const withoutSearchText: PaletteFile[] = [{ id: "f1", name: NIRCAM_NAME, filter: "F200W" }];
    expect(optionLabels(paletteAfterTyping(withoutSearchText, "nrca1"))).toEqual([NIRCAM_NAME]);
    expect(optionLabels(paletteAfterTyping(withoutSearchText, "f200"))).toEqual([NIRCAM_NAME]);
    expect(options(paletteAfterTyping(withoutSearchText, "nircam"))).toHaveLength(0);
  });

  it("says when a listed file is hidden by the Files filter", () => {
    const files: PaletteFile[] = [
      { id: "a", name: "a_s3d.fits", filter: "F290LP", hidden: true },
      { id: "b", name: "b_s3d.fits", hidden: true },
      { id: "c", name: "c_x1d.fits", filter: "F290LP" },
    ];
    const hints = paletteAfterTyping(files, "")
      .filter((el) => el.props.className === "ab-cmdp-hint")
      .map((el) => el.props.children);
    expect(hints).toEqual(["F290LP · hidden by filter", "hidden by filter", "F290LP"]);
    expect(HIDDEN_FILE_HINT).toBe("hidden by filter");
  });
});
