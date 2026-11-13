import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactNode } from "react";
import type { ProcessedResult } from "../../../shared/types/preview";

const screen = vi.hoisted(() => ({ processed: null as ProcessedResult | null }));

vi.mock("react", async (original) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await original()));
vi.mock("../../../context/PreviewContext", () => ({ useRenderContext: () => ({ processed: screen.processed }) }));

import { hooks } from "../../../test/hookRuntime";
import { cubeFrameRequesters, formatFrameLabel, type CubeFrameRequester } from "../../../utils/cubeNavigation";
import ViewerFrameStepper, { FrameKeysFocusRing } from "../ViewerFrameStepper";
import { useViewerFrameKeys, useViewerFrameStep } from "../useViewerFrameKeys";

type Props = Record<string, unknown> & { children?: ReactNode };

const CUBE = "C:/data/jw01266_nirspec_s3d.fits";
const OTHER = "C:/data/other_s3d.fits";
const TOTAL = 40;
const stepper = (ViewerFrameStepper as unknown as { type: (props: Props) => ReactNode }).type;
const channelLabel = (idx: number) => formatFrameLabel(idx, TOTAL, null, "");

function elements(node: ReactNode): Props[] {
  if (Array.isArray(node)) return node.flatMap(elements);
  if (!isValidElement<Props>(node)) return [];
  return [node.props, ...elements(node.props.children)];
}

function textOf(node: ReactNode): string {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  return isValidElement<Props>(node) ? textOf(node.props.children) : "";
}

function cubeFrame(frameIndex: number | undefined, inputPath = CUBE, kind: ProcessedResult["kind"] = "cube"): ProcessedResult {
  return { fitsPath: null, previewUrl: "asset://frame.png", dimensions: null, label: `ch ${frameIndex}`, kind, inputPath, frameIndex };
}

const request = vi.fn<(idx: number) => void>();
const unregister: (() => void)[] = [];

function mountCubeTool(over: Partial<CubeFrameRequester> = {}): void {
  unregister.push(
    cubeFrameRequesters.register({
      filePath: CUBE,
      totalFrames: TOTAL,
      playing: false,
      failure: null,
      labelFor: channelLabel,
      stepOrigin: (displayed) => displayed,
      request,
      ...over,
    }),
  );
}

function viewer() {
  return hooks.render(() => {
    const step = useViewerFrameStep();
    return { step, keys: useViewerFrameKeys(step) };
  }, {}) as { step: ReturnType<typeof useViewerFrameStep>; keys: ReturnType<typeof useViewerFrameKeys> };
}

function stepperOn(frameIndex: number): Props[] {
  screen.processed = cubeFrame(frameIndex);
  const { step } = viewer();
  if (step === null) throw new Error("no frame stepper for a cube frame of a mounted Cube tool");
  return elements(stepper({ step }));
}

const byTitle = (nodes: Props[], title: string) => nodes.find((p) => p.title === title);
const press = (node: Props | undefined) => (node?.onClick as () => void)();
const readoutOf = (nodes: Props[]) => nodes.find((p) => typeof p.children === "string" && p.onClick === undefined && typeof p.title === "string");

function key(name: string, shiftKey = false) {
  const viewport = {};
  return { key: name, shiftKey, target: viewport, currentTarget: viewport, preventDefault: vi.fn() };
}

beforeEach(() => {
  hooks.mount();
  request.mockReset();
  screen.processed = null;
});

afterEach(() => {
  hooks.mount();
  while (unregister.length > 0) unregister.pop()?.();
});

describe("ViewerFrameStepper", () => {
  it("requests the displayed frame minus one and plus one through the Cube tool's frame request", () => {
    mountCubeTool();
    const nodes = stepperOn(5);
    expect(byTitle(nodes, "Previous frame")?.["aria-label"]).toBe("Previous frame");
    expect(byTitle(nodes, "Next frame")?.["aria-label"]).toBe("Next frame");
    press(byTitle(nodes, "Previous frame"));
    press(byTitle(nodes, "Next frame"));
    expect(request.mock.calls).toEqual([[4], [6]]);
  });

  it("reads the displayed frame with the Cube tool's label for it, so it counts frames like the frame labels beside it", () => {
    mountCubeTool();
    const nodes = stepperOn(5);
    const readout = readoutOf(nodes);
    expect(readout?.children).toBe("ch 5 (0–39)");
    expect(readout?.title).toMatch(/^ch 5 \(0–39\)\. /);
    const order = nodes.map((p) => (p.title === "Previous frame" ? "prev" : p.title === "Next frame" ? "next" : p === readout ? "readout" : null)).filter(Boolean);
    expect(order).toEqual(["prev", "readout", "next"]);
  });

  it("follows a 1-based ramp label for the same frame index", () => {
    mountCubeTool({ labelFor: (idx) => `group ${idx + 1} of ${TOTAL}` });
    expect(readoutOf(stepperOn(5))?.children).toBe("group 6 of 40");
  });

  it("clamps at both ends, so no request leaves the cube", () => {
    mountCubeTool();
    const first = stepperOn(0);
    expect(byTitle(first, "Previous frame")?.disabled).toBe(true);
    press(byTitle(first, "Next frame"));
    const last = stepperOn(TOTAL - 1);
    expect(byTitle(last, "Next frame")?.disabled).toBe(true);
    press(byTitle(last, "Previous frame"));
    const beyond = stepperOn(TOTAL + 17);
    expect(readoutOf(beyond)?.children).toBe(channelLabel(TOTAL - 1));
    expect(byTitle(beyond, "Next frame")?.disabled).toBe(true);
    press(byTitle(beyond, "Previous frame"));
    expect(request.mock.calls).toEqual([[1], [TOTAL - 2], [TOTAL - 2]]);
  });

  it("shows no stepper without a cube frame on screen or without a mounted Cube tool for that cube", () => {
    mountCubeTool();
    expect(viewer().step).toBeNull();
    screen.processed = cubeFrame(undefined);
    expect(viewer().step, "a moment map or collapse has no frame index").toBeNull();
    screen.processed = cubeFrame(5, CUBE, "processing");
    expect(viewer().step).toBeNull();
    screen.processed = cubeFrame(5, OTHER);
    expect(viewer().step, "the frame of a cube whose Cube tool is not mounted").toBeNull();
    screen.processed = cubeFrame(5);
    expect(viewer().step?.frame).toBe(5);
    expect(viewer().keys).toBeDefined();
  });

  it("gives no stepper and no frame keys on a single-frame cube", () => {
    mountCubeTool({ totalFrames: 1 });
    screen.processed = cubeFrame(0);
    const { step, keys } = viewer();
    expect(step).toBeNull();
    expect(keys).toBeUndefined();
  });

  it("uses the Cube tool mounted last for the cube on screen", () => {
    const older = vi.fn<(idx: number) => void>();
    mountCubeTool({ request: older });
    mountCubeTool();
    press(byTitle(stepperOn(5), "Next frame"));
    expect(request).toHaveBeenCalledWith(6);
    expect(older).not.toHaveBeenCalled();
  });

  it("disables both buttons and ignores the arrow keys while the Cube tool plays the frames", () => {
    mountCubeTool({ playing: true });
    const nodes = stepperOn(5);
    expect(byTitle(nodes, "Previous frame")?.disabled).toBe(true);
    expect(byTitle(nodes, "Next frame")?.disabled).toBe(true);
    expect(nodes.some((p) => typeof p.title === "string" && p.title.includes("pause playback"))).toBe(true);
    expect(readoutOf(nodes)?.title).toBe("Playing in the Spectra tool: pause playback to step frames");
    const arrow = key("ArrowRight");
    viewer().keys?.onKeyDown(arrow as never);
    expect(request).not.toHaveBeenCalled();
  });
});

describe("ViewerFrameStepper with a frame that failed to load", () => {
  it("marks the frame the Cube tool could not load and steps on from it instead of requesting it again", () => {
    mountCubeTool({ failure: { idx: 6, message: "disk unavailable" }, stepOrigin: () => 6 });
    const nodes = stepperOn(5);
    const readout = readoutOf(nodes);
    expect(readout?.children).toBe("Failed: ch 6 (0–39)");
    expect(readout?.title).toContain("Could not load ch 6 (0–39): disk unavailable");
    expect(readout?.title).toMatch(/Retry it in the Spectra tool$/);
    expect(readout?.className).toContain("text-red-400");
    press(byTitle(nodes, "Next frame"));
    press(byTitle(nodes, "Previous frame"));
    viewer().keys?.onKeyDown(key("ArrowRight") as never);
    expect(request.mock.calls).toEqual([[7], [5], [7]]);
  });

  it("keeps the displayed frame unmarked when only the full-resolution load of that same frame failed (control)", () => {
    mountCubeTool({ failure: { idx: 5, message: "disk unavailable" } });
    const readout = readoutOf(stepperOn(5));
    expect(readout?.children).toBe("ch 5 (0–39)");
    expect(readout?.className).not.toContain("text-red-400");
  });

  it("steps from the frame the Cube tool is still loading, like its own buttons, so a second press moves on", () => {
    mountCubeTool({ stepOrigin: () => 7 });
    const nodes = stepperOn(5);
    press(byTitle(nodes, "Next frame"));
    viewer().keys?.onKeyDown(key("ArrowLeft") as never);
    expect(request.mock.calls).toEqual([[8], [6]]);
  });
});

describe("viewer frame keys", () => {
  it("make the viewer a focusable group that announces its arrow-key shortcuts and hands its outline to the focus ring", () => {
    mountCubeTool();
    screen.processed = cubeFrame(5);
    const keys = viewer().keys;
    expect(keys?.tabIndex).toBe(0);
    expect(keys?.role).toBe("group");
    expect(keys?.["aria-label"]).toBe("Cube frame viewer");
    expect(keys?.["aria-keyshortcuts"]).toBe("ArrowLeft ArrowRight Shift+ArrowLeft Shift+ArrowRight");
    expect(keys?.style).toEqual({ outline: "none" });
  });

  it("step the displayed frame with ArrowLeft and ArrowRight, by ten with Shift, through the same request", () => {
    mountCubeTool();
    screen.processed = cubeFrame(5);
    const keys = viewer().keys;
    const right = key("ArrowRight");
    keys?.onKeyDown(right as never);
    keys?.onKeyDown(key("ArrowLeft") as never);
    keys?.onKeyDown(key("ArrowRight", true) as never);
    keys?.onKeyDown(key("ArrowLeft", true) as never);
    expect(request.mock.calls).toEqual([[6], [4], [15], [0]]);
    expect(right.preventDefault).toHaveBeenCalledTimes(1);
  });

  it("leave other keys to the viewer and keep the arrows from scrolling at the last frame", () => {
    mountCubeTool();
    screen.processed = cubeFrame(TOTAL - 1);
    const keys = viewer().keys;
    const other = key("a");
    keys?.onKeyDown(other as never);
    expect(other.preventDefault).not.toHaveBeenCalled();
    const right = key("ArrowRight");
    keys?.onKeyDown(right as never);
    expect(right.preventDefault).toHaveBeenCalledTimes(1);
    expect(request).not.toHaveBeenCalled();
    keys?.onKeyDown(key("ArrowLeft") as never);
    expect(request).toHaveBeenCalledWith(TOTAL - 2);
  });

  it("leave the arrow keys pressed in a control inside the viewer, such as a display limit field, to that control", () => {
    mountCubeTool();
    screen.processed = cubeFrame(5);
    const fromField = { ...key("ArrowRight"), target: { tagName: "INPUT" } };
    viewer().keys?.onKeyDown(fromField as never);
    expect(fromField.preventDefault).not.toHaveBeenCalled();
    expect(request).not.toHaveBeenCalled();
  });
});

describe("FrameKeysFocusRing", () => {
  it("draws the ring as a layer over the whole viewer, above the z-10 toolbar, status bar and label, shown only on keyboard focus of the viewer", () => {
    const ring = FrameKeysFocusRing() as { props: Props };
    expect(ring.props["aria-hidden"]).toBe("true");
    expect(String(ring.props.className).split(" ")).toEqual(expect.arrayContaining(["hidden", "group-focus-visible/frame-keys:block"]));
    expect(ring.props.style).toMatchObject({ position: "absolute", inset: 0, zIndex: 11, pointerEvents: "none", outline: "2px solid var(--ab-accent)", outlineOffset: -2 });
    expect(textOf(ring.props.children)).toBe("");
  });
});
