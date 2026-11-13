import { afterEach, describe, expect, it, vi } from "vitest";
import {
  domSectionHold,
  followTableRoute,
  holdSectionAtStart,
  scrollToSectionWhenMounted,
  SECTION_HOLD_MS,
  SECTION_MOUNT_TRIES,
  tableRouteHost,
  type ScrollTarget,
  type SectionHold,
} from "../tableRoutes";

function routeDeps() {
  const calls: string[] = [];
  const openTool = vi.fn((tool: string) => void calls.push(`open:${tool}`));
  const scrollTo = vi.fn((sectionId: string) => void calls.push(`scroll:${sectionId}`));
  return { calls, deps: { openTool, scrollTo } };
}

function frameQueue() {
  const queue: (() => void)[] = [];
  return {
    queue,
    nextFrame: (run: () => void) => void queue.push(run),
    runNext: () => queue.shift()?.(),
  };
}

describe("followTableRoute", () => {
  it("opens the Photometry section of the kind when a file is on screen", () => {
    const lightCurve = routeDeps();
    expect(followTableRoute("light_curve", true, lightCurve.deps)).toBeNull();
    expect(lightCurve.deps.openTool.mock.calls).toEqual([["photometry"]]);
    expect(lightCurve.deps.scrollTo.mock.calls).toEqual([["analysis-series"]]);
    expect(lightCurve.calls).toEqual(["open:photometry", "scroll:analysis-series"]);
    const catalogue = routeDeps();
    expect(followTableRoute("catalogue", true, catalogue.deps)).toBeNull();
    expect(catalogue.calls).toEqual(["open:photometry", "scroll:analysis-compare"]);
  });

  it("says the table is kept for its section instead of opening a tool that cannot show", () => {
    const { calls, deps } = routeDeps();
    expect(followTableRoute("light_curve", false, deps)).toBe(
      "Kept for the Series section of Photometry: select an image or a spectrum, then open Photometry to see the light curve",
    );
    expect(followTableRoute("catalogue", false, deps)).toBe(
      "Kept for the Compare section of Photometry: select the image to compare, then open Photometry to use the catalogue",
    );
    expect(calls).toEqual([]);
  });
});

describe("scrollToSectionWhenMounted", () => {
  it("scrolls to the section once it is mounted and gives up after the tries", () => {
    const frames = frameQueue();
    const target: ScrollTarget = { scrollIntoView: vi.fn() };
    const found = [null, null, target];
    const find = vi.fn<(id: string) => ScrollTarget | null>(() => found.shift() ?? null);
    scrollToSectionWhenMounted("analysis-series", find, frames.nextFrame);
    expect(target.scrollIntoView).not.toHaveBeenCalled();
    frames.runNext();
    frames.runNext();
    frames.runNext();
    expect(target.scrollIntoView).toHaveBeenCalledTimes(1);
    expect(target.scrollIntoView).toHaveBeenCalledWith({ block: "start" });
    expect(find.mock.calls.every(([id]) => id === "analysis-series")).toBe(true);
    expect(frames.queue).toHaveLength(0);

    const missing = frameQueue();
    const never = vi.fn(() => null);
    scrollToSectionWhenMounted("analysis-compare", never, missing.nextFrame, 3);
    let ran = 0;
    while (missing.queue.length > 0) {
      missing.runNext();
      ran++;
    }
    expect(ran).toBe(3);
    expect(never).toHaveBeenCalledTimes(3);

    const defaults = frameQueue();
    const absent = vi.fn(() => null);
    scrollToSectionWhenMounted("analysis-compare", absent, defaults.nextFrame);
    let frames30 = 0;
    while (defaults.queue.length > 0) {
      defaults.runNext();
      frames30++;
    }
    expect(frames30).toBe(30);
    expect(absent).toHaveBeenCalledTimes(30);
  });
});

function toolLayout(viewport: number, heights: Record<string, number>) {
  const order = Object.keys(heights);
  const resize: (() => void)[] = [];
  const user: (() => void)[] = [];
  const timers: { ms: number; run: () => void }[] = [];
  const view = { scrollTop: 0 };
  const topOf = (id: string) => order.slice(0, order.indexOf(id)).reduce((sum, key) => sum + heights[key], 0);
  const maxScroll = () => Math.max(0, order.reduce((sum, key) => sum + heights[key], 0) - viewport);
  const section = (id: string): ScrollTarget => ({
    scrollIntoView: () => {
      view.scrollTop = Math.min(topOf(id), maxScroll());
    },
  });
  const atTop = () => order.find((id) => topOf(id) + heights[id] > view.scrollTop) ?? null;
  const grow = (id: string, height: number) => {
    heights[id] = height;
    view.scrollTop = Math.min(view.scrollTop, maxScroll());
    for (const run of [...resize]) run();
  };
  const hold: SectionHold = {
    onResize: (run) => {
      resize.push(run);
      return () => void resize.splice(resize.indexOf(run), 1);
    },
    onUserInput: (run) => {
      user.push(run);
      return () => void user.splice(user.indexOf(run), 1);
    },
    after: (ms, run) => {
      const timer = { ms, run };
      timers.push(timer);
      return () => void timers.splice(timers.indexOf(timer), 1);
    },
  };
  return { view, section, atTop, grow, hold, resize, user, timers };
}

function routeTo(layout: ReturnType<typeof toolLayout>, sectionId: string) {
  const frames = frameQueue();
  scrollToSectionWhenMounted(sectionId, layout.section, frames.nextFrame, SECTION_MOUNT_TRIES, (target) => holdSectionAtStart(target, layout.hold));
  frames.runNext();
}

describe("holdSectionAtStart", () => {
  it("brings Compare to the start once its table makes it tall enough, instead of leaving Resolution on screen", () => {
    const layout = toolLayout(600, { "analysis-depth": 300, "analysis-resolution": 500, "analysis-compare": 160 });
    routeTo(layout, "analysis-compare");
    expect(layout.atTop()).toBe("analysis-resolution");
    layout.grow("analysis-compare", 700);
    expect(layout.atTop()).toBe("analysis-compare");
    expect(layout.view.scrollTop).toBe(800);
  });

  it("keeps Series at the start while the sections above it grow", () => {
    const layout = toolLayout(600, { "analysis-photometry": 120, "analysis-table": 80, "analysis-series": 400, "analysis-forced": 900 });
    routeTo(layout, "analysis-series");
    expect(layout.view.scrollTop).toBe(200);
    layout.grow("analysis-table", 450);
    expect(layout.view.scrollTop).toBe(570);
    expect(layout.atTop()).toBe("analysis-series");
  });

  it("lets the user take over and ends after the hold time", () => {
    const layout = toolLayout(600, { "analysis-resolution": 500, "analysis-compare": 160 });
    routeTo(layout, "analysis-compare");
    expect(layout.timers.map((t) => t.ms)).toEqual([SECTION_HOLD_MS]);
    layout.user[0]();
    expect([layout.resize.length, layout.user.length, layout.timers.length]).toEqual([0, 0, 0]);
    layout.grow("analysis-compare", 700);
    expect(layout.atTop()).toBe("analysis-resolution");

    const timed = toolLayout(600, { "analysis-resolution": 500, "analysis-compare": 160 });
    routeTo(timed, "analysis-compare");
    timed.timers[0].run();
    expect([timed.resize.length, timed.user.length, timed.timers.length]).toEqual([0, 0, 0]);
  });
});

describe("domSectionHold", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("watches the tool content and its dock host, stops on wheel, pointer, key or touch input, and times out", () => {
    const observed: unknown[] = [];
    const disconnect = vi.fn();
    vi.stubGlobal(
      "ResizeObserver",
      class {
        observe(el: unknown) {
          observed.push(el);
        }
        disconnect = disconnect;
      },
    );
    const added: [string, unknown, unknown][] = [];
    const removed: [string, unknown, unknown][] = [];
    const clearTimeout = vi.fn();
    vi.stubGlobal("window", {
      addEventListener: (type: string, run: unknown, options: unknown) => void added.push([type, run, options]),
      removeEventListener: (type: string, run: unknown, options: unknown) => void removed.push([type, run, options]),
      setTimeout: vi.fn(() => 7),
      clearTimeout,
    });
    const content = { id: "content" };
    const host = { id: "host" };
    const section = { parentElement: content, closest: vi.fn(() => host) } as unknown as Element;
    const hold = domSectionHold(section);
    const stopResize = hold.onResize(() => {});
    expect(observed).toEqual([content, host]);
    expect(section.closest).toHaveBeenCalledWith("[data-dock-host]");
    stopResize();
    expect(disconnect).toHaveBeenCalledTimes(1);
    const run = () => {};
    const stopInput = hold.onUserInput(run);
    expect(added.map(([type, fn]) => [type, fn === run])).toEqual([["wheel", true], ["pointerdown", true], ["keydown", true], ["touchstart", true]]);
    expect(added.every(([, , options]) => (options as AddEventListenerOptions).capture === true)).toBe(true);
    stopInput();
    expect(removed.map(([type, fn]) => [type, fn === run])).toEqual([["wheel", true], ["pointerdown", true], ["keydown", true], ["touchstart", true]]);
    const stopTimer = hold.after(SECTION_HOLD_MS, run);
    expect(window.setTimeout).toHaveBeenCalledWith(run, SECTION_HOLD_MS);
    stopTimer();
    expect(clearTimeout).toHaveBeenCalledWith(7);
  });
});

describe("tableRouteHost", () => {
  it("puts back the image that was on screen for a catalogue, and an image or a spectrum for a light curve", () => {
    expect(tableRouteHost("catalogue", { id: "img", status: "done" })).toBe("img");
    expect(tableRouteHost("light_curve", { id: "img", status: "done" })).toBe("img");
    expect(tableRouteHost("light_curve", { id: "x1d", status: "spectrum" })).toBe("x1d");
  });

  it("keeps the table for later when nothing fit was on screen before", () => {
    expect(tableRouteHost("catalogue", { id: "x1d", status: "spectrum" })).toBeNull();
    expect(tableRouteHost("catalogue", { id: "t", status: "table" })).toBeNull();
    expect(tableRouteHost("light_curve", { id: "bad", status: "error" })).toBeNull();
    expect(tableRouteHost("light_curve", { id: "q", status: "queued" })).toBeNull();
    expect(tableRouteHost("catalogue", undefined)).toBeNull();
    expect(tableRouteHost("light_curve", null)).toBeNull();
  });
});
