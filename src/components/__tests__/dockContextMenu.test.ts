import { describe, expect, it, vi } from "vitest";
import { createElement, isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { DockMenuList } from "../dock/DockContextMenu";
import LiveRegion from "../dock/LiveRegion";
import { announce, moveAnnouncement } from "../dock/useDockDrag";
import { DEFAULT_DOCK_LAYOUT, DOCK_ANCHORS, dockReducer, type DockAnchor, type DockToolId } from "../../utils/dockLayout";

interface MenuProps {
  tool: DockToolId;
  anchor: DockAnchor;
  index: number;
  count: number;
  onMove: ReturnType<typeof vi.fn<(anchor: DockAnchor) => void>>;
  onReorder: ReturnType<typeof vi.fn<(delta: -1 | 1) => void>>;
  onReset: ReturnType<typeof vi.fn<() => void>>;
  onShowTips: ReturnType<typeof vi.fn<() => void>>;
  onClose: ReturnType<typeof vi.fn<() => void>>;
}

function menuProps(over: Partial<MenuProps> = {}): MenuProps {
  return {
    tool: "image",
    anchor: "right-top",
    index: 1,
    count: 4,
    onMove: vi.fn<(anchor: DockAnchor) => void>(),
    onReorder: vi.fn<(delta: -1 | 1) => void>(),
    onReset: vi.fn<() => void>(),
    onShowTips: vi.fn<() => void>(),
    onClose: vi.fn<() => void>(),
    ...over,
  };
}

function render(props: MenuProps): string {
  return renderToStaticMarkup(createElement(DockMenuList, props));
}

interface RenderedButton {
  attrs: string;
  text: string;
}

function buttons(html: string, role: string): RenderedButton[] {
  return [...html.matchAll(/<button\b([^>]*)>([\s\S]*?)<\/button>/g)]
    .filter((m) => m[1].includes(`role="${role}"`))
    .map((m) => ({ attrs: m[1], text: m[2].replace(/<[^>]*>/g, "").trim() }));
}

function byText(list: RenderedButton[], text: string): RenderedButton {
  const found = list.find((b) => b.text === text);
  if (!found) throw new Error(`no item "${text}" in ${list.map((b) => b.text).join(", ")}`);
  return found;
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

function tree(props: MenuProps): ReactElement<AnyProps> {
  return DockMenuList(props) as ReactElement<AnyProps>;
}

function activate(props: MenuProps, role: string, text: string): void {
  const el = findElement(tree(props), (p) => p.role === role && textOf(p.children).trim() === text);
  (el.props.onClick as () => void)();
}

interface FakeItem {
  focus: ReturnType<typeof vi.fn>;
}

function pressKey(props: MenuProps, key: string, focusedIndex: number) {
  const menu = findElement(tree(props), (p) => p.role === "menu");
  const items: FakeItem[] = Array.from({ length: 7 }, () => ({ focus: vi.fn() }));
  const event = {
    key,
    target: focusedIndex >= 0 ? items[focusedIndex] : {},
    currentTarget: { querySelectorAll: () => items },
    preventDefault: vi.fn(),
    stopPropagation: vi.fn(),
  };
  (menu.props.onKeyDown as (e: typeof event) => void)(event);
  return { items, event };
}

describe("DockMenuList markup", () => {
  it("is a menu labelled after the tool and carries the data hook", () => {
    const html = render(menuProps());
    expect(html).toMatch(/role="menu"/);
    expect(html).toMatch(/aria-label="Image options"/);
    expect(html).toMatch(/data-dock-menu=""/);
    expect(render(menuProps({ tool: "config" }))).toMatch(/aria-label="Settings options"/);
  });

  it("lists the four anchors as radio items in the owner's order and wording", () => {
    const radios = buttons(render(menuProps()), "menuitemradio");
    expect(radios.map((b) => b.text)).toEqual(["Left Top", "Bottom Left", "Right Top", "Bottom Right"]);
  });

  it("groups the radio items under a non-focusable Move to heading", () => {
    const html = render(menuProps());
    expect(html).toMatch(/role="group"[^>]*aria-label="Move to"|aria-label="Move to"[^>]*role="group"/);
    expect(html).toMatch(/<div[^>]*aria-hidden="true"[^>]*>Move to<\/div>/);
  });

  it("checks exactly the current anchor", () => {
    const labels: Record<DockAnchor, string> = { "left-top": "Left Top", "left-bottom": "Bottom Left", "right-top": "Right Top", "right-bottom": "Bottom Right" };
    for (const anchor of DOCK_ANCHORS) {
      const radios = buttons(render(menuProps({ anchor })), "menuitemradio");
      const checked = radios.filter((b) => b.attrs.includes('aria-checked="true"')).map((b) => b.text);
      const unchecked = radios.filter((b) => b.attrs.includes('aria-checked="false"')).length;
      expect(checked).toEqual([labels[anchor]]);
      expect(unchecked).toBe(3);
    }
  });

  it("disables Move up at the first position only", () => {
    const first = buttons(render(menuProps({ index: 0, count: 4 })), "menuitem");
    expect(byText(first, "Move up").attrs).toContain('aria-disabled="true"');
    expect(byText(first, "Move down").attrs).not.toContain('aria-disabled="true"');
  });

  it("disables Move down at the last position only", () => {
    const last = buttons(render(menuProps({ index: 3, count: 4 })), "menuitem");
    expect(byText(last, "Move down").attrs).toContain('aria-disabled="true"');
    expect(byText(last, "Move up").attrs).not.toContain('aria-disabled="true"');
  });

  it("enables both moves in the middle and disables both for a lone tool", () => {
    const middle = buttons(render(menuProps({ index: 1, count: 3 })), "menuitem");
    expect(middle.filter((b) => b.attrs.includes('aria-disabled="true"'))).toEqual([]);
    const lone = buttons(render(menuProps({ index: 0, count: 1 })), "menuitem");
    expect(byText(lone, "Move up").attrs).toContain('aria-disabled="true"');
    expect(byText(lone, "Move down").attrs).toContain('aria-disabled="true"');
  });

  it("ends with Reset layout and Show layout tips after separators", () => {
    const html = render(menuProps());
    expect(buttons(html, "menuitem").map((b) => b.text)).toEqual(["Move up", "Move down", "Reset layout", "Show layout tips"]);
    expect(html.match(/role="separator"/g)?.length).toBe(2);
  });

  it("keeps every item out of the tab order", () => {
    const html = render(menuProps());
    const items = [...buttons(html, "menuitemradio"), ...buttons(html, "menuitem")];
    expect(items).toHaveLength(8);
    for (const item of items) {
      expect(item.attrs).toContain('tabindex="-1"');
      expect(item.attrs).toContain('type="button"');
    }
  });
});

describe("DockMenuList activation", () => {
  it("moves to another anchor without closing on its own", () => {
    const props = menuProps({ anchor: "right-top" });
    activate(props, "menuitemradio", "Bottom Left");
    expect(props.onMove).toHaveBeenCalledWith("left-bottom");
    expect(props.onClose).not.toHaveBeenCalled();
  });

  it("only closes when the checked anchor is chosen", () => {
    const props = menuProps({ anchor: "right-top" });
    activate(props, "menuitemradio", "Right Top");
    expect(props.onMove).not.toHaveBeenCalled();
    expect(props.onClose).toHaveBeenCalledTimes(1);
  });

  it("ignores a disabled Move up and reorders down", () => {
    const props = menuProps({ index: 0, count: 3 });
    activate(props, "menuitem", "Move up");
    expect(props.onReorder).not.toHaveBeenCalled();
    activate(props, "menuitem", "Move down");
    expect(props.onReorder).toHaveBeenCalledWith(1);
  });

  it("ignores a disabled Move down and reorders up", () => {
    const props = menuProps({ index: 2, count: 3 });
    activate(props, "menuitem", "Move down");
    expect(props.onReorder).not.toHaveBeenCalled();
    activate(props, "menuitem", "Move up");
    expect(props.onReorder).toHaveBeenCalledWith(-1);
  });

  it("resets the layout", () => {
    const props = menuProps();
    activate(props, "menuitem", "Reset layout");
    expect(props.onReset).toHaveBeenCalledTimes(1);
    expect(props.onShowTips).not.toHaveBeenCalled();
  });

  it("shows the layout tips", () => {
    const props = menuProps();
    activate(props, "menuitem", "Show layout tips");
    expect(props.onShowTips).toHaveBeenCalledTimes(1);
    expect(props.onReset).not.toHaveBeenCalled();
  });
});

describe("DockMenuList roving focus", () => {
  it("moves down and wraps from the last item to the first", () => {
    expect(pressKey(menuProps(), "ArrowDown", 2).items[3].focus).toHaveBeenCalled();
    expect(pressKey(menuProps(), "ArrowDown", 6).items[0].focus).toHaveBeenCalled();
  });

  it("moves up and wraps from the first item to the last", () => {
    expect(pressKey(menuProps(), "ArrowUp", 3).items[2].focus).toHaveBeenCalled();
    expect(pressKey(menuProps(), "ArrowUp", 0).items[6].focus).toHaveBeenCalled();
  });

  it("jumps with Home and End", () => {
    expect(pressKey(menuProps(), "Home", 4).items[0].focus).toHaveBeenCalled();
    expect(pressKey(menuProps(), "End", 1).items[6].focus).toHaveBeenCalled();
  });

  it("consumes navigation keys so global shortcuts do not see them", () => {
    const { event } = pressKey(menuProps(), "ArrowDown", 0);
    expect(event.preventDefault).toHaveBeenCalled();
    expect(event.stopPropagation).toHaveBeenCalled();
  });

  it("closes on Escape and Tab", () => {
    const escape = menuProps();
    pressKey(escape, "Escape", 1);
    expect(escape.onClose).toHaveBeenCalledTimes(1);
    const tab = menuProps();
    const { event } = pressKey(tab, "Tab", 1);
    expect(tab.onClose).toHaveBeenCalledTimes(1);
    expect(event.preventDefault).toHaveBeenCalled();
  });

  it("keeps other keys from global shortcuts without acting on them", () => {
    for (const key of ["a", "k", "?", "Shift", "ContextMenu"]) {
      const props = menuProps();
      const { items, event } = pressKey(props, key, 1);
      expect(event.stopPropagation).toHaveBeenCalled();
      expect(items.some((item) => item.focus.mock.calls.length > 0)).toBe(false);
      expect(event.preventDefault).not.toHaveBeenCalled();
      expect(props.onClose).not.toHaveBeenCalled();
    }
  });

  it("keeps key releases from global shortcuts", () => {
    const menu = findElement(tree(menuProps()), (p) => p.role === "menu");
    const event = { key: "Shift", stopPropagation: vi.fn(), preventDefault: vi.fn() };
    (menu.props.onKeyUp as (e: typeof event) => void)(event);
    expect(event.stopPropagation).toHaveBeenCalled();
    expect(event.preventDefault).not.toHaveBeenCalled();
  });
});

describe("DockMenuList native menu and focus", () => {
  function menuHandler(props: MenuProps, name: string): (e: unknown) => void {
    const menu = findElement(tree(props), (p) => p.role === "menu");
    return menu.props[name] as (e: unknown) => void;
  }

  it("suppresses the native context menu, including the one the Menu key fires on keyup", () => {
    const event = { preventDefault: vi.fn() };
    menuHandler(menuProps(), "onContextMenu")(event);
    expect(event.preventDefault).toHaveBeenCalledTimes(1);
  });

  it("keeps focus on the current item when the menu is pressed", () => {
    const event = { preventDefault: vi.fn() };
    menuHandler(menuProps(), "onMouseDown")(event);
    expect(event.preventDefault).toHaveBeenCalledTimes(1);
  });

  it("closes when focus leaves the menu", () => {
    const outside = {};
    const toOutside = menuProps();
    menuHandler(toOutside, "onBlur")({ relatedTarget: outside, currentTarget: { contains: (n: unknown) => n !== outside } });
    expect(toOutside.onClose).toHaveBeenCalledTimes(1);
    const toNothing = menuProps();
    menuHandler(toNothing, "onBlur")({ relatedTarget: null, currentTarget: { contains: () => true } });
    expect(toNothing.onClose).toHaveBeenCalledTimes(1);
  });

  it("stays open while focus moves between its items", () => {
    const props = menuProps();
    menuHandler(props, "onBlur")({ relatedTarget: {}, currentTarget: { contains: () => true } });
    expect(props.onClose).not.toHaveBeenCalled();
  });
});

describe("moveAnnouncement", () => {
  it("names the target anchor with the owner's labels after a cross-anchor move", () => {
    const after = dockReducer(DEFAULT_DOCK_LAYOUT, { type: "move", tool: "image", anchor: "right-bottom" });
    expect(moveAnnouncement(DEFAULT_DOCK_LAYOUT, after, "image")).toBe("Image moved to Bottom Right");
    const left = dockReducer(DEFAULT_DOCK_LAYOUT, { type: "move", tool: "config", anchor: "left-bottom" });
    expect(moveAnnouncement(DEFAULT_DOCK_LAYOUT, left, "config")).toBe("Settings moved to Bottom Left");
  });

  it("says up or down for a reorder inside the anchor", () => {
    const up = dockReducer(DEFAULT_DOCK_LAYOUT, { type: "reorder", anchor: "right-top", from: 5, to: 0 });
    expect(moveAnnouncement(DEFAULT_DOCK_LAYOUT, up, "processing")).toBe("Processing moved up");
    const down = dockReducer(DEFAULT_DOCK_LAYOUT, { type: "reorder", anchor: "right-top", from: 0, to: 3 });
    expect(moveAnnouncement(DEFAULT_DOCK_LAYOUT, down, "headers")).toBe("Headers moved down");
  });

  it("stays silent when the tool did not move", () => {
    expect(moveAnnouncement(DEFAULT_DOCK_LAYOUT, DEFAULT_DOCK_LAYOUT, "image")).toBeNull();
    const other = dockReducer(DEFAULT_DOCK_LAYOUT, { type: "reorder", anchor: "right-bottom", from: 0, to: 2 });
    expect(moveAnnouncement(DEFAULT_DOCK_LAYOUT, other, "image")).toBeNull();
  });
});

describe("LiveRegion", () => {
  it("is a polite, visually hidden region with the data hook", () => {
    const html = renderToStaticMarkup(createElement(LiveRegion));
    expect(html).toMatch(/aria-live="polite"/);
    expect(html).toMatch(/data-dock-live=""/);
    expect(html).toMatch(/class="sr-only"/);
  });

  it("shows the latest announcement", () => {
    announce("Image moved to Bottom Right");
    expect(renderToStaticMarkup(createElement(LiveRegion))).toContain("Image moved to Bottom Right");
    announce("Layout reset");
    const html = renderToStaticMarkup(createElement(LiveRegion));
    expect(html).toContain("Layout reset");
    expect(html).not.toContain("Image moved");
  });
});
