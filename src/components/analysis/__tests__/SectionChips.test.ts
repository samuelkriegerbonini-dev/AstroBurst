import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { hooks } from "../../../test/hookRuntime";
import { elementsOf, textOf } from "../../../test/elementTree";
import { ANALYSIS_SECTION, type AnalysisSection } from "../../../utils/analysisSections";
import SectionChips, { SectionTipNote } from "../SectionChips";
import { focusSectionHelp, useSectionTip, type SectionTips } from "../useSectionTip";

vi.mock("react", async (original) => {
  const { reactWithFakeHooks } = await import("../../../test/hookRuntime");
  return reactWithFakeHooks(await original<Record<string, unknown>>());
});

type Node = { type: unknown; props: Record<string, unknown> };

interface PanelProps {
  label: string;
  sections: readonly AnalysisSection[];
  tips?: SectionTips;
}

const REGIONS = ANALYSIS_SECTION.regions.id;

const props: PanelProps = {
  label: "Image",
  sections: [ANALYSIS_SECTION.statistics, ANALYSIS_SECTION.pixels, ANALYSIS_SECTION.regions, ANALYSIS_SECTION.profiles],
  tips: { [REGIONS]: { title: "Measure a region", body: "Choose a shape in the viewer, then draw on the image." } },
};

function Panel({ label, sections, tips }: PanelProps) {
  const tip = useSectionTip(sections, tips);
  return { chips: SectionChips({ label, sections, tip }), note: SectionTipNote({ tip, sectionId: REGIONS }) };
}

const scrollIntoView = vi.fn();
const focus = vi.fn();
const querySelector = vi.fn((_selector: string) => ({ focus }));
const storage = new Map<string, string>();

function view(next: PanelProps = props) {
  const out = hooks.render(Panel, next) as { chips: ReactNode; note: ReactNode };
  const chips = elementsOf(out.chips);
  const note = elementsOf(out.note);
  return { chips, note, all: [...chips, ...note] };
}

const notes = (nodes: Node[]) => nodes.filter((node) => node.props.role === "note");
const helpOf = (nodes: Node[]) => nodes.find((node) => node.props["data-section-help"] === REGIONS);
const navs = (nodes: Node[]) => nodes.filter((node) => node.type === "nav");

function keyDown(node: Node | undefined, key: string) {
  const event = { key, preventDefault: vi.fn(), stopPropagation: vi.fn() };
  (node?.props.onKeyDown as (e: typeof event) => void)(event);
  return event;
}

function click(nodes: Node[], label: string) {
  const button = nodes.find((node) => node.type === "button" && (node.props["aria-label"] === label || textOf(node.props.children) === label));
  if (!button) throw new Error(`Missing button: ${label}`);
  (button.props.onClick as () => void)();
}

beforeEach(() => {
  hooks.mount();
  storage.clear();
  scrollIntoView.mockClear();
  focus.mockClear();
  querySelector.mockClear();
  vi.stubGlobal("document", { getElementById: vi.fn(() => ({ style: {}, scrollIntoView })), querySelector });
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { callback(0); return 1; });
  vi.stubGlobal("localStorage", { getItem: (key: string) => storage.get(key) ?? null, setItem: (key: string, value: string) => storage.set(key, value) });
});

afterEach(() => {
  hooks.mount();
  vi.unstubAllGlobals();
});

describe("Section guidance", () => {
  it("stays hidden on initial render and appears only after selecting its section", () => {
    const initial = view();
    expect(notes(initial.all)).toHaveLength(0);
    click(initial.all, "Regions");
    const note = notes(view().all)[0];
    expect(note?.props["aria-label"]).toBe("Measure a region");
    expect(textOf(note?.props.children)).toContain("Choose a shape in the viewer");
    expect(document.getElementById).toHaveBeenCalledWith(REGIONS);
    expect(scrollIntoView).toHaveBeenCalledOnce();
  });

  it("renders the tip inside its own section, outside the sticky chip bar", () => {
    click(view().all, "Regions");
    const open = view();
    expect(notes(open.chips)).toHaveLength(0);
    expect(notes(open.note)).toHaveLength(1);
    expect(navs(open.chips)).toHaveLength(1);
  });

  it("remembers dismissal across remounts while allowing explicit help to reopen it", () => {
    click(view().all, "Regions");
    click(view().all, "Dismiss Measure a region tip");
    expect(notes(view().all)).toHaveLength(0);
    hooks.mount();
    click(view().all, "Regions");
    expect(notes(view().all)).toHaveLength(0);
    click(view().all, "Help: Measure a region");
    expect(notes(view().all)).toHaveLength(1);
  });

  it("closes the previous guidance when a different section is selected", () => {
    click(view().all, "Regions");
    expect(notes(view().all)).toHaveLength(1);
    click(view().all, "Stats");
    expect(notes(view().all)).toHaveLength(0);
  });

  it("keeps a tip that was shown closed after a round trip through another chip, for this mount only", () => {
    click(view().all, "Regions");
    click(view().all, "Stats");
    click(view().all, "Regions");
    expect(notes(view().all)).toHaveLength(0);
    expect(storage.size).toBe(0);
    click(view().all, "Help: Measure a region");
    expect(notes(view().all)).toHaveLength(1);
    hooks.mount();
    click(view().all, "Regions");
    expect(notes(view().all)).toHaveLength(1);
  });

  it("keeps the tip open when its own chip is selected again", () => {
    click(view().all, "Regions");
    click(view().all, "Regions");
    expect(notes(view().all)).toHaveLength(1);
  });

  it("keeps dismissal usable when browser storage is unavailable", () => {
    vi.stubGlobal("localStorage", { getItem: () => { throw new Error("unavailable"); }, setItem: () => { throw new Error("unavailable"); } });
    click(view().all, "Regions");
    click(view().all, "Dismiss Measure a region tip");
    click(view().all, "Regions");
    expect(notes(view().all)).toHaveLength(0);
    click(view().all, "Help: Measure a region");
    expect(notes(view().all)).toHaveLength(1);
  });

  it("hides guidance when the selected section is no longer available", () => {
    click(view().all, "Regions");
    expect(notes(view().all)).toHaveLength(1);
    const withoutRegions = view({ ...props, sections: [ANALYSIS_SECTION.statistics, ANALYSIS_SECTION.pixels, ANALYSIS_SECTION.profiles, ANALYSIS_SECTION.histogram] });
    expect(navs(withoutRegions.chips)).toHaveLength(1);
    expect(notes(withoutRegions.all)).toHaveLength(0);
  });

  it("connects help controls to their expanded note and restores collapsed state after dismissal", () => {
    const initial = view();
    expect(helpOf(initial.all)?.props["aria-expanded"]).toBe(false);
    click(initial.all, "Help: Measure a region");
    const open = view();
    const note = notes(open.all)[0];
    const help = helpOf(open.all);
    expect(help?.props["aria-expanded"]).toBe(true);
    expect(note?.props.id).toEqual(expect.any(String));
    expect(help?.props["aria-controls"]).toBe(note?.props.id);
    const body = open.all.find((node) => node.type === "p" && textOf(node.props.children).includes("Choose a shape"));
    expect(help?.props["aria-describedby"]).toBe(body?.props.id);
    click(open.all, "Dismiss Measure a region tip");
    expect(helpOf(view().all)?.props["aria-expanded"]).toBe(false);
  });

  it.each([true, false])("honors reduced motion %s when navigating to a section", (reduceMotion) => {
    vi.stubGlobal("window", { matchMedia: vi.fn(() => ({ matches: reduceMotion })) });
    click(view().all, "Regions");
    expect(window.matchMedia).toHaveBeenCalledWith("(prefers-reduced-motion: reduce)");
    expect(scrollIntoView).toHaveBeenCalledWith({ block: "start", behavior: reduceMotion ? "auto" : "smooth" });
  });
});

describe("Section chips by section count", () => {
  it("shows the chip row from two sections, with or without tips", () => {
    const pair = [ANALYSIS_SECTION.statistics, ANALYSIS_SECTION.regions];
    expect(navs(view({ label: "Image", sections: pair }).chips)).toHaveLength(1);
    hooks.mount();
    const withTips = view({ ...props, sections: pair });
    expect(navs(withTips.chips)).toHaveLength(1);
    expect(helpOf(withTips.chips)).toBeDefined();
    expect(helpOf(withTips.note)).toBeUndefined();
  });

  it("puts the help of a single section in the section itself instead of a one-chip row", () => {
    const single = { ...props, sections: [ANALYSIS_SECTION.regions] };
    const closed = view(single);
    expect(closed.chips).toHaveLength(0);
    expect(helpOf(closed.note)?.props["aria-label"]).toBe("Help: Measure a region");
    click(closed.note, "Help: Measure a region");
    expect(notes(view(single).note)).toHaveLength(1);
    hooks.mount();
    expect(view({ label: "Short panel", sections: single.sections }).all).toHaveLength(0);
  });
});

describe("Section guidance keyboard and announcements", () => {
  it("dismisses the open tip on Escape inside the note and keeps the key from the region tool", () => {
    click(view().all, "Regions");
    const event = keyDown(view().note[0], "Escape");
    expect(event.preventDefault).toHaveBeenCalledOnce();
    expect(event.stopPropagation).toHaveBeenCalledOnce();
    expect(notes(view().all)).toHaveLength(0);
    expect(storage.get(`ab.section-tip.${REGIONS}`)).toBe("dismissed");
  });

  it("dismisses the open tip on Escape from the chip row without moving focus", () => {
    click(view().all, "Regions");
    const event = keyDown(view().chips[0], "Escape");
    expect(event.stopPropagation).toHaveBeenCalledOnce();
    expect(notes(view().all)).toHaveLength(0);
    expect(focus).not.toHaveBeenCalled();
  });

  it("lets Escape through when no tip is open and ignores other keys", () => {
    const closed = view();
    for (const root of [closed.chips[0], closed.note[0]]) expect(keyDown(root, "Escape").stopPropagation).not.toHaveBeenCalled();
    click(view().all, "Regions");
    expect(keyDown(view().note[0], "Enter").stopPropagation).not.toHaveBeenCalled();
    expect(notes(view().all)).toHaveLength(1);
  });

  it("toggles the note from its help button like a disclosure", () => {
    click(view().all, "Help: Measure a region");
    expect(helpOf(view().all)?.props["aria-expanded"]).toBe(true);
    click(view().all, "Help: Measure a region");
    const closed = view();
    expect(notes(closed.all)).toHaveLength(0);
    expect(helpOf(closed.all)?.props["aria-expanded"]).toBe(false);
  });

  it("labels the help button Help: <title> and announces the note through a polite live region", () => {
    const help = helpOf(view().all);
    expect(help?.props["aria-label"]).toBe("Help: Measure a region");
    expect(help?.props.title).toBe("Help: Measure a region");
    const live = () => view().note.filter((node) => node.props["aria-live"] === "polite");
    expect(live()).toHaveLength(1);
    click(view().all, "Regions");
    expect(notes(elementsOf(live()[0]?.props.children))).toHaveLength(1);
  });
});

describe("Section guidance focus", () => {
  it("returns focus to the help button after the tip is dismissed", () => {
    click(view().all, "Regions");
    click(view().all, "Dismiss Measure a region tip");
    expect(querySelector).toHaveBeenCalledWith(`[data-section-help="${REGIONS}"]`);
    expect(focus).toHaveBeenCalledWith({ preventScroll: true });
  });

  it("focuses the section's help button through focusSectionHelp and tolerates a missing root", () => {
    const button = { focus: vi.fn() };
    const root = { querySelector: vi.fn(() => button) };
    focusSectionHelp(root as unknown as ParentNode, REGIONS);
    expect(root.querySelector).toHaveBeenCalledWith(`[data-section-help="${REGIONS}"]`);
    expect(button.focus).toHaveBeenCalledWith({ preventScroll: true });
    expect(() => focusSectionHelp(null, REGIONS)).not.toThrow();
  });
});
