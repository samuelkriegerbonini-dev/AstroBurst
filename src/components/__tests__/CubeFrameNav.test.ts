import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactNode } from "react";

vi.mock("react", async (original) => (await import("../../test/hookRuntime")).reactWithFakeHooks(await original()));
vi.mock("../../services/cube", () => ({ getCubeFrame: vi.fn() }));

import { hooks } from "../../test/hookRuntime";
import { createFramePublishGate, cubeFrameOutputPaths, cubeFrameRequesters } from "../../utils/cubeNavigation";
import CubeFrameNav, { frameStepTarget, type FrameLoader } from "../CubeFrameNav";

type Props = Record<string, unknown> & { children?: ReactNode };

const FILE = "C:/data/cube.fits";
const TOTAL = 40;
const nav = (CubeFrameNav as unknown as { type: (props: Props) => ReactNode }).type;

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

const onFrameRequest = vi.fn<(idx: number) => void>();
const onFrameChange = vi.fn<(previewUrl: string, frameIndex: number, fitsPath?: string) => void>();
const frameLoader = vi.fn<FrameLoader>();

function render(over: Props = {}): Props[] {
  return elements(
    hooks.render(nav, { filePath: FILE, totalFrames: TOTAL, frame: 0, requestSeq: 0, onFrameRequest, onFrameChange, frameLoader, ...over }) as ReactNode,
  );
}

const byLabel = (nodes: Props[], label: string) => nodes.find((p) => p["aria-label"] === label);
const byText = (nodes: Props[], text: string) => nodes.find((p) => p.onClick !== undefined && textOf(p.children).trim() === text);
const press = (node: Props | undefined) => (node?.onClick as () => void)();
const retryButton = (nodes: Props[]) => nodes.find((p) => p.title === "Load this frame again");
function failureText(nodes: Props[]): string | null {
  const texts = nodes.map((p) => textOf(p.children)).filter((text) => text.startsWith("Frame load failed"));
  return texts.length > 0 ? texts[texts.length - 1] : null;
}

function attach(node: Props | undefined, current: unknown): void {
  const ref = node?.ref as { current: unknown } | undefined;
  if (ref) ref.current = current;
}

function focusedElement(focused: boolean): { ownerDocument: { activeElement: unknown } } {
  const element = { ownerDocument: { activeElement: null as unknown } };
  element.ownerDocument.activeElement = focused ? element : {};
  return element;
}

async function failFrame(frame: number, extra: Props = {}): Promise<Props[]> {
  frameLoader.mockRejectedValueOnce(new Error("disk unavailable"));
  render({ frame, requestSeq: 0, ...extra });
  render({ frame, requestSeq: 1, ...extra });
  await settle();
  return render({ frame, requestSeq: 1, ...extra });
}

async function settle(): Promise<void> {
  for (let i = 0; i < 6; i++) await Promise.resolve();
}

beforeEach(() => {
  hooks.mount();
  onFrameRequest.mockReset();
  onFrameChange.mockReset();
  frameLoader.mockReset();
  vi.useFakeTimers();
  vi.spyOn(console, "error").mockImplementation(() => {});
});

afterEach(() => {
  hooks.mount();
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("frameStepTarget", () => {
  it("steps by one inside the cube and wraps at both ends only with Loop on", () => {
    expect(frameStepTarget(5, TOTAL, 1, false)).toBe(6);
    expect(frameStepTarget(5, TOTAL, -1, true)).toBe(4);
    expect(frameStepTarget(TOTAL - 1, TOTAL, 1, true)).toBe(0);
    expect(frameStepTarget(TOTAL - 1, TOTAL, 1, false)).toBeNull();
    expect(frameStepTarget(0, TOTAL, -1, true)).toBe(TOTAL - 1);
    expect(frameStepTarget(0, TOTAL, -1, false)).toBeNull();
    expect(frameStepTarget(0, 1, -1, true)).toBeNull();
    expect(frameStepTarget(0, 1, 1, true)).toBeNull();
  });
});

describe("CubeFrameNav steps with Loop", () => {
  it("keeps Next enabled on the last frame with Loop on and wraps to the first", () => {
    const next = byLabel(render({ frame: TOTAL - 1, loop: true }), "Next frame");
    expect(next?.disabled).toBe(false);
    expect(next?.title).toBe("Next frame (wraps to the first)");
    press(next);
    expect(onFrameRequest).toHaveBeenCalledWith(0);
  });

  it("keeps Previous enabled on the first frame with Loop on and wraps to the last", () => {
    const prev = byLabel(render({ frame: 0, loop: true }), "Previous frame");
    expect(prev?.disabled).toBe(false);
    expect(prev?.title).toBe("Previous frame (wraps to the last)");
    press(prev);
    expect(onFrameRequest).toHaveBeenCalledWith(TOTAL - 1);
  });

  it("steps normally inside the cube with Loop on", () => {
    const nodes = render({ frame: 5, loop: true });
    expect(byLabel(nodes, "Next frame")?.title).toBe("Next frame");
    expect(byLabel(nodes, "Previous frame")?.title).toBe("Previous frame");
    press(byLabel(nodes, "Next frame"));
    press(byLabel(nodes, "Previous frame"));
    expect(onFrameRequest.mock.calls).toEqual([[6], [4]]);
  });

  it("stops at both ends with Loop off", () => {
    expect(byLabel(render({ frame: TOTAL - 1, loop: false }), "Next frame")?.disabled).toBe(true);
    expect(byLabel(render({ frame: 0, loop: false }), "Previous frame")?.disabled).toBe(true);
    expect(byLabel(render({ frame: 0, loop: false }), "Next frame")?.title).toBe("Next frame");
  });
});

describe("CubeFrameNav frame requests from the viewer", () => {
  const requesterFor = (path: string) => cubeFrameRequesters.get().findLast((entry) => entry.filePath === path);

  it("registers its frame request for its cube while mounted, clamped to the cube, and withdraws it on unmount", () => {
    render({ frame: 5 });
    const requester = requesterFor(FILE);
    expect(requester?.totalFrames).toBe(TOTAL);
    expect(requester?.playing).toBe(false);
    requester?.request(6);
    expect(onFrameRequest).toHaveBeenLastCalledWith(6);
    requester?.request(TOTAL + 5);
    expect(onFrameRequest).toHaveBeenLastCalledWith(TOTAL - 1);
    requester?.request(-3);
    expect(onFrameRequest).toHaveBeenLastCalledWith(0);
    hooks.mount();
    expect(requesterFor(FILE)).toBeUndefined();
  });

  it("commits the frame it already holds instead of requesting it again, like its own buttons", () => {
    const gate = createFramePublishGate();
    render({ frame: 5, publishGate: gate });
    gate.supersede();
    requesterFor(FILE)?.request(5);
    expect(onFrameRequest).not.toHaveBeenCalled();
    expect(gate.isCurrent()).toBe(true);
  });

  it("registers nothing for a single-frame cube", () => {
    render({ totalFrames: 1 });
    expect(requesterFor(FILE)).toBeUndefined();
  });

  it("reports playback, so the viewer holds its stepper while the frames play", () => {
    press(byLabel(render({ frame: 5 }), "Play all frames"));
    const playing = render({ frame: 5 });
    expect(requesterFor(FILE)?.playing).toBe(true);
    press(byLabel(playing, "Pause playback"));
    render({ frame: 5 });
    expect(requesterFor(FILE)?.playing).toBe(false);
    expect(cubeFrameRequesters.get().filter((entry) => entry.filePath === FILE)).toHaveLength(1);
  });

  it("hands the viewer the label it shows for a frame, the panel's own labels when it has them", () => {
    render({ frame: 5 });
    expect(requesterFor(FILE)?.labelFor(5)).toBe("ch 5 (0–39)");
    hooks.mount();
    render({ frame: 5, labelFor: (idx: number) => `group ${idx + 1} of ${TOTAL}` });
    expect(requesterFor(FILE)?.labelFor(5)).toBe("group 6 of 40");
  });

  it("lets the viewer step from the displayed frame until a frame is requested through it, then from its own cursor", () => {
    frameLoader.mockImplementation(() => new Promise(() => {}));
    render({ frame: 0 });
    expect(requesterFor(FILE)?.stepOrigin(5), "a cursor nothing moved yet, like a reopened Ramp panel at 0").toBe(5);
    requesterFor(FILE)?.request(6);
    expect(onFrameRequest).toHaveBeenLastCalledWith(6);
    render({ frame: 6, requestSeq: 1 });
    expect(requesterFor(FILE)?.stepOrigin(5), "the frame it is still loading").toBe(6);
  });

  it("hands the viewer the failure of the frame it stands on and drops it once it moves on", async () => {
    await failFrame(6);
    expect(requesterFor(FILE)?.failure).toEqual({ idx: 6, message: "disk unavailable" });
    expect(requesterFor(FILE)?.stepOrigin(5), "the failed frame, so Next goes past it").toBe(6);
    frameLoader.mockImplementation(() => new Promise(() => {}));
    render({ frame: 7, requestSeq: 2 });
    expect(requesterFor(FILE)?.failure).toBeNull();
    expect(requesterFor(FILE)?.stepOrigin(5)).toBe(7);
  });

  it("starts over from the displayed frame after its cube changes", () => {
    frameLoader.mockImplementation(() => new Promise(() => {}));
    render({ frame: 0 });
    requesterFor(FILE)?.request(6);
    render({ frame: 6, requestSeq: 1 });
    render({ filePath: "C:/data/other.fits", frame: 0, requestSeq: 1 });
    expect(requesterFor("C:/data/other.fits")?.stepOrigin(5)).toBe(5);
  });
});

describe("CubeFrameNav failed frame", () => {
  it("offers Retry after a failed frame and reloads that frame with its FITS", async () => {
    frameLoader.mockRejectedValueOnce(new Error("disk unavailable"));
    render({ frame: 3, requestSeq: 0 });
    render({ frame: 3, requestSeq: 1 });
    await settle();
    const failed = render({ frame: 3, requestSeq: 1 });
    expect(failed.some((p) => textOf(p.children).includes("Frame load failed: disk unavailable"))).toBe(true);
    const retry = byText(failed, "Retry");
    expect(retry).toBeDefined();
    const before = frameLoader.mock.calls.length;
    const paths = cubeFrameOutputPaths(FILE, 3);
    frameLoader.mockResolvedValueOnce({ output_path: paths.png, fits_path: paths.fits });
    press(retry);
    expect(frameLoader.mock.calls.length).toBe(before + 1);
    expect(frameLoader).toHaveBeenLastCalledWith(FILE, 3, paths.png, paths.fits);
    await settle();
    const loaded = render({ frame: 3, requestSeq: 1 });
    expect(loaded.some((p) => textOf(p.children).includes("Frame load failed"))).toBe(false);
    expect(byText(loaded, "Retry")).toBeUndefined();
  });

  it("keeps Retry mounted as an aria-disabled Retrying... button until the reload settles, so focus stays on it", async () => {
    const failed = await failFrame(3);
    let rejectRetry: (e: Error) => void = () => {};
    frameLoader.mockImplementationOnce(() => new Promise((_resolve, reject) => (rejectRetry = reject)));
    press(retryButton(failed));
    const during = render({ frame: 3, requestSeq: 1 });
    const busy = retryButton(during);
    expect(textOf(busy?.children as ReactNode)).toBe("Retrying...");
    expect(busy?.["aria-disabled"]).toBe(true);
    expect(failureText(during)).toBe("Frame load failed: disk unavailable");
    const calls = frameLoader.mock.calls.length;
    press(busy);
    expect(frameLoader.mock.calls.length, "a second press while retrying starts no load").toBe(calls);
    rejectRetry(new Error("still unavailable"));
    await settle();
    const failedAgain = render({ frame: 3, requestSeq: 1 });
    expect(textOf(retryButton(failedAgain)?.children as ReactNode)).toBe("Retry");
    expect(retryButton(failedAgain)?.["aria-disabled"]).toBeUndefined();
    expect(failureText(failedAgain)).toBe("Frame load failed: still unavailable");
  });

  it("moves focus to the frame slider when the reload succeeds while Retry has focus", async () => {
    const failed = await failFrame(3);
    const focusSlider = vi.fn();
    attach(byLabel(failed, "Cube frame"), { focus: focusSlider });
    attach(retryButton(failed), focusedElement(true));
    const paths = cubeFrameOutputPaths(FILE, 3);
    frameLoader.mockResolvedValueOnce({ output_path: paths.png, fits_path: paths.fits });
    press(retryButton(failed));
    await settle();
    expect(focusSlider).toHaveBeenCalledTimes(1);
    expect(retryButton(render({ frame: 3, requestSeq: 1 }))).toBeUndefined();
  });

  it("leaves focus alone when the reload succeeds after focus moved off Retry (control)", async () => {
    const failed = await failFrame(3);
    const focusSlider = vi.fn();
    attach(byLabel(failed, "Cube frame"), { focus: focusSlider });
    attach(retryButton(failed), focusedElement(false));
    const paths = cubeFrameOutputPaths(FILE, 3);
    frameLoader.mockResolvedValueOnce({ output_path: paths.png, fits_path: paths.fits });
    press(retryButton(failed));
    await settle();
    expect(focusSlider).not.toHaveBeenCalled();
    expect(failureText(render({ frame: 3, requestSeq: 1 }))).toBeNull();
  });

  it("ties the failure to its frame, so a cached frame shown meanwhile offers no Retry and the failed frame loads again on return", async () => {
    frameLoader.mockImplementation(async (_path, _idx, png, fits) => ({ output_path: png, fits_path: fits ?? null }));
    const p10 = cubeFrameOutputPaths(FILE, 10);
    render({ frame: 10, requestSeq: 0 });
    render({ frame: 10, requestSeq: 1 });
    await settle();
    await vi.advanceTimersByTimeAsync(400);
    await settle();
    expect(frameLoader).toHaveBeenCalledWith(FILE, 10, p10.png, p10.fits);
    let reject11: (e: Error) => void = () => {};
    frameLoader.mockImplementationOnce(() => new Promise((_resolve, reject) => (reject11 = reject)));
    render({ frame: 11, requestSeq: 2 });
    await settle();
    render({ frame: 10, requestSeq: 3 });
    await settle();
    reject11(new Error("disk unavailable"));
    await settle();
    const onTen = render({ frame: 10, requestSeq: 3 });
    expect(failureText(onTen), "the failure of frame 11 is not shown over frame 10").toBeNull();
    expect(retryButton(onTen)).toBeUndefined();
    const p11 = cubeFrameOutputPaths(FILE, 11);
    render({ frame: 11, requestSeq: 4 });
    expect(frameLoader).toHaveBeenLastCalledWith(FILE, 11, p11.png, undefined);
    await settle();
    expect(failureText(render({ frame: 11, requestSeq: 4 }))).toBeNull();
  });

  it("commits the publish gate on Retry, so the reloaded frame shows over a result published after the failure", async () => {
    const gate = createFramePublishGate();
    const failed = await failFrame(3, { publishGate: gate });
    gate.supersede();
    const paths = cubeFrameOutputPaths(FILE, 3);
    frameLoader.mockResolvedValueOnce({ output_path: paths.png, fits_path: paths.fits });
    onFrameChange.mockClear();
    press(retryButton(failed));
    await settle();
    expect(onFrameChange).toHaveBeenCalledWith(paths.png, 3, paths.fits);
    expect(gate.isCurrent()).toBe(true);
    expect(failureText(render({ frame: 3, requestSeq: 1, publishGate: gate }))).toBeNull();
  });
});
