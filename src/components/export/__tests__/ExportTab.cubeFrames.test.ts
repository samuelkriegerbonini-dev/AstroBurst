import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactNode } from "react";

const { current, getCubeFrame } = vi.hoisted(() => ({
  current: { path: "C:/data/a_cube.fits" },
  getCubeFrame: vi.fn(),
}));

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));

vi.mock("../../../context/PreviewContext", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../context/PreviewContext")>()),
  useFileContext: () => ({ file: { id: current.path, name: "cube.fits", path: current.path, result: { is_rgb: false, previewUrl: null } } }),
  useHistContext: () => ({ stfParams: { shadow: 0, midtone: 0.5, highlight: 1 }, histData: null }),
  useDisplayContext: () => ({ display: {} }),
  useRgbContext: () => ({ rgbChannels: null }),
  useDisplayedImage: () => ({ path: current.path, isProcessed: false, label: null, previewOnly: false }),
  useRenderContext: () => ({ processed: null }),
  useCubeContext: () => ({ isCube: true, cubeDims: { width: 8, height: 8, frames: 3 } }),
}));

vi.mock("../../../context/CompositeContext", async (importOriginal) => {
  const stf = { shadow: 0, midtone: 0.5, highlight: 1 };
  return {
    ...(await importOriginal<typeof import("../../../context/CompositeContext")>()),
    useCompositePreview: () => ({ compositePreviewUrl: null, isShowingComposite: false, compositeVersion: 0, canShowParked: false }),
    useCompositeStf: () => ({ compositeStfR: stf, compositeStfG: stf, compositeStfB: stf, compositeStfLinked: true }),
  };
});

vi.mock("../../../services/cube", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/cube")>()),
  getCubeFrame,
}));

vi.mock("../../../infrastructure/tauri", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../infrastructure/tauri")>()),
  getExportDir: () => Promise.resolve("C:/export"),
}));

import { hooks } from "../../../test/hookRuntime";
import ExportTab from "../ExportTab";
import { TauriCommandError } from "../../../shared/types/errors";

const ExportTabInner = (ExportTab as unknown as { type: (props: object) => ReactNode }).type;

interface Node {
  type: unknown;
  props: Record<string, unknown> & { children?: ReactNode };
}

function nodes(node: ReactNode): Node[] {
  if (Array.isArray(node)) return node.flatMap(nodes);
  if (!isValidElement<Node["props"]>(node)) return [];
  return [{ type: node.type, props: node.props }, ...nodes(node.props.children)];
}

function textOf(node: ReactNode): string {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  return isValidElement<Node["props"]>(node) ? textOf(node.props.children) : "";
}

function view(): Node[] {
  return nodes(hooks.render(ExportTabInner, {}) as ReactNode);
}

function button(tree: Node[], label: string): Node | undefined {
  return tree.find((n) => n.type === "button" && textOf(n.props.children).includes(label));
}

function exportAll(): Promise<void> {
  const start = button(view(), "Export All Frames");
  expect(start, "Export All Frames button").toBeDefined();
  return (start!.props.onClick as () => Promise<void>)();
}

function resultText(tree: Node[]): string | null {
  const box = tree.find((n) => n.type === "div" && String(n.props.className).includes("font-mono px-2 py-1 rounded"));
  return box ? textOf(box.props.children) : null;
}

function statusRegion(tree: Node[]): Node | undefined {
  return tree.find((n) => n.props.role === "status");
}

function attach(node: Node | undefined, element: object): void {
  const ref = node?.props.ref as { current: unknown } | undefined;
  if (ref) ref.current = element;
}

function focusedElement(): object {
  const element = { ownerDocument: { activeElement: null as unknown } };
  element.ownerDocument.activeElement = element;
  return element;
}

function unfocusedElement(): object {
  return { ownerDocument: { activeElement: null } };
}

function holdFrames(): { finish: () => void } {
  const held = { finish: () => {} };
  getCubeFrame.mockImplementation(() => new Promise((resolve) => {
    held.finish = () => resolve({ output_path: "C:/export/frame.png" });
  }));
  return held;
}

beforeEach(() => {
  hooks.mount();
  getCubeFrame.mockReset();
  current.path = "C:/data/a_cube.fits";
});

afterEach(() => {
  hooks.mount();
  vi.restoreAllMocks();
});

describe("Export All Frames on a cube", () => {
  it("stops before the next frame when Stop is pressed, and says how many frames it wrote", async () => {
    let finishFrame = () => {};
    getCubeFrame.mockImplementation(() => new Promise((resolve) => {
      finishFrame = () => resolve({ output_path: "C:/export/a_cube_frame_0000.png" });
    }));
    const run = exportAll();
    await vi.waitFor(() => expect(getCubeFrame).toHaveBeenCalledTimes(1));
    const stop = button(view(), "Stop");
    expect(stop, "Stop button while exporting").toBeDefined();
    (stop!.props.onClick as () => void)();
    finishFrame();
    await run;
    expect(getCubeFrame).toHaveBeenCalledTimes(1);
    const tree = view();
    expect(resultText(tree)).toContain("1/3 frames exported");
    expect(resultText(tree)).toContain("stopped");
    expect(button(tree, "Stop")).toBeUndefined();
  });

  it("shows a finished export only under the cube it ran on", async () => {
    getCubeFrame.mockResolvedValue({ output_path: "C:/export/frame.png" });
    await exportAll();
    expect(getCubeFrame).toHaveBeenCalledTimes(3);
    expect(resultText(view())).toContain("3/3 frames exported");
    current.path = "C:/data/b_cube.fits";
    expect(resultText(view())).toBeNull();
    current.path = "C:/data/a_cube.fits";
    expect(resultText(view())).toContain("3/3 frames exported");
  });

  it("says why the export stopped without the error class name", async () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    getCubeFrame
      .mockResolvedValueOnce({ output_path: "C:/export/frame.png" })
      .mockRejectedValueOnce(new TauriCommandError("get_cube_frame", "frame 1 is outside the cube"));
    await exportAll();
    const text = resultText(view());
    expect(text).toContain("1/3 frames exported");
    expect(text).toContain("frame 1 is outside the cube");
    expect(text).not.toContain("TauriCommandError");
  });
});

describe("Stop and the end of a cube export for keyboard and screen-reader users", () => {
  it("keeps Stop on screen as Stopping... after the click, without the native disabled state that drops focus", async () => {
    const frame = holdFrames();
    const run = exportAll();
    await vi.waitFor(() => expect(getCubeFrame).toHaveBeenCalledTimes(1));
    (button(view(), "Stop")!.props.onClick as () => void)();
    const stopping = button(view(), "Stopping...");
    expect(stopping, "Stopping... button after the click").toBeDefined();
    expect(stopping!.props["aria-disabled"]).toBe(true);
    expect(stopping!.props.disabled).toBeFalsy();
    frame.finish();
    await run;
  });

  it("returns focus to Export All Frames when a stopped run ends while Stop has focus", async () => {
    const frame = holdFrames();
    const run = exportAll();
    await vi.waitFor(() => expect(getCubeFrame).toHaveBeenCalledTimes(1));
    const tree = view();
    const focus = vi.fn();
    attach(button(tree, "Stop"), focusedElement());
    attach(button(tree, "Exporting"), { focus });
    (button(tree, "Stop")!.props.onClick as () => void)();
    frame.finish();
    await run;
    view();
    expect(focus).toHaveBeenCalledTimes(1);
  });

  it("returns focus to Export All Frames when the export finishes on its own while Stop has focus", async () => {
    let finishLast = () => {};
    getCubeFrame
      .mockResolvedValueOnce({ output_path: "C:/export/frame0.png" })
      .mockResolvedValueOnce({ output_path: "C:/export/frame1.png" })
      .mockImplementationOnce(() => new Promise((resolve) => {
        finishLast = () => resolve({ output_path: "C:/export/frame2.png" });
      }));
    const run = exportAll();
    await vi.waitFor(() => expect(getCubeFrame).toHaveBeenCalledTimes(3));
    const tree = view();
    const focus = vi.fn();
    attach(button(tree, "Stop"), focusedElement());
    attach(button(tree, "Exporting"), { focus });
    finishLast();
    await run;
    expect(resultText(view())).toContain("3/3 frames exported");
    expect(focus).toHaveBeenCalledTimes(1);
  });

  it("leaves focus where it is when the run ends while Stop does not have it", async () => {
    const frame = holdFrames();
    const run = exportAll();
    await vi.waitFor(() => expect(getCubeFrame).toHaveBeenCalledTimes(1));
    const tree = view();
    const focus = vi.fn();
    attach(button(tree, "Stop"), unfocusedElement());
    attach(button(tree, "Exporting"), { focus });
    (button(tree, "Stop")!.props.onClick as () => void)();
    frame.finish();
    await run;
    view();
    expect(focus).not.toHaveBeenCalled();
  });

  it("announces the outcome through a status region that is already mounted when the result arrives", async () => {
    const frame = holdFrames();
    const run = exportAll();
    await vi.waitFor(() => expect(getCubeFrame).toHaveBeenCalledTimes(1));
    const during = statusRegion(view());
    expect(during, "status region while the export runs").toBeDefined();
    expect(textOf(during!.props.children)).toBe("");
    (button(view(), "Stop")!.props.onClick as () => void)();
    frame.finish();
    await run;
    const after = statusRegion(view());
    expect(after, "status region after the run").toBeDefined();
    expect(textOf(after!.props.children)).toContain("1/3 frames exported (stopped)");
  });
});
