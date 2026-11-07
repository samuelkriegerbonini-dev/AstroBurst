import { afterEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type Key, type ReactNode } from "react";

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));

vi.mock("../../../context/PreviewContext", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../context/PreviewContext")>()),
  useDoneFilesContext: () => ({ doneFiles: [] }),
  useFileContext: () => ({ file: null }),
  useRenderActions: () => ({ publishProcessed: () => {}, currentFileKey: () => null }),
}));

vi.mock("../../../context/CompositeContext", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../context/CompositeContext")>()),
  useCompositeActions: () => ({ park: () => {} }),
}));

vi.mock("../../../context/ComposeWizardContext", async (importOriginal) => {
  const { INITIAL_STATE } = await import("../../../utils/wizard");
  return {
    ...(await importOriginal<typeof import("../../../context/ComposeWizardContext")>()),
    useComposeWizardContext: () => ({ state: INITIAL_STATE, dispatch: () => {}, getState: () => INITIAL_STATE, setActiveStep: () => {} }),
  };
});

vi.mock("../../../hooks/useCompositeMode", () => ({ useCompositeMode: () => false }));

vi.mock("../../../infrastructure/tauri", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../infrastructure/tauri")>()),
  getOutputDir: () => Promise.resolve("C:/out"),
  getPreviewUrl: () => Promise.resolve(null),
}));

import { hooks } from "../../../test/hookRuntime";
import StackingTab from "../StackingTab";

const StackingTabInner = (StackingTab as unknown as { type: (props: object) => ReactNode }).type;

interface Node {
  type: unknown;
  key: Key | null;
  props: Record<string, unknown> & { children?: ReactNode };
}

function nodes(node: ReactNode): Node[] {
  if (Array.isArray(node)) return node.flatMap(nodes);
  if (!isValidElement<Node["props"]>(node)) return [];
  return [{ type: node.type, key: node.key, props: node.props }, ...nodes(node.props.children)];
}

function view(): Node[] {
  return nodes(hooks.render(StackingTabInner, {}) as ReactNode);
}

function pills(tree: Node[]): Node[] {
  const row = tree.find((n) => n.props.role === "group" && n.props["aria-label"] === "Stacking sections");
  expect(row, "Stacking sections row").toBeDefined();
  return nodes(row!.props.children).filter((n) => n.type === "button");
}

function pill(tree: Node[], section: string): Node {
  const found = pills(tree).find((p) => p.key === section);
  expect(found, section).toBeDefined();
  return found!;
}

function currentSections(tree: Node[]): (Key | null)[] {
  return pills(tree).filter((p) => p.props["aria-current"] === "true").map((p) => p.key);
}

afterEach(() => hooks.mount());

describe("Stacking section pills", () => {
  it("name their row and mark only the open section as current, not as pressed toggles", () => {
    hooks.mount();
    const tree = view();
    expect(pills(tree)).toHaveLength(7);
    expect(currentSections(tree)).toEqual(["calibrate"]);
    expect(pills(tree).filter((p) => "aria-pressed" in p.props)).toEqual([]);
    (pill(tree, "stack").props.onClick as () => void)();
    expect(currentSections(view())).toEqual(["stack"]);
  });

  it("title each section with what it does, like the Processing pills", () => {
    hooks.mount();
    expect(Object.fromEntries(pills(view()).map((p) => [p.key, p.props.title]))).toEqual({
      calibrate: "Subtract bias and dark, divide by flat",
      cosmetic: "Repair hot and cold pixels",
      subframe: "Measure frame quality and choose which to stack",
      stack: "Register and combine the selected frames",
      pipeline: "Calibrate, cosmetic, stack and compose in one run",
      drizzle_rgb: "Drizzle three filters straight into an RGB composite",
      mosaic: "Join overlapping tiles into one image",
    });
  });

  it("say in text, not only with the dot, that a calibrated frame is ready to stack", () => {
    hooks.mount();
    const calibration = view().find((n) => typeof n.props.onCalibrationDone === "function");
    expect(calibration, "calibration panel").toBeDefined();
    (calibration!.props.onCalibrationDone as (...args: unknown[]) => void)(
      { fits_path: "C:/out/light_cal.fits", previewUrl: null, dimensions: [8, 8], has_bias: true, has_dark: true, has_flat: true },
      { darkPaths: [], flatPaths: [], biasPaths: [], flatDarkPaths: [] },
      "C:/data/light.fits",
      null,
    );
    const tree = view();
    const spans = nodes(pill(tree, "stack").props.children).filter((n) => n.type === "span");
    expect(spans.find((s) => String(s.props.className).startsWith("ab-processing-pill-dot"))?.props.title).toBe("Calibrated frame ready to stack");
    expect(spans.filter((s) => s.props.className === "sr-only").map((s) => s.props.children)).toEqual([", calibrated frame ready"]);
    expect(nodes(pill(tree, "calibrate").props.children).filter((n) => n.props.className === "sr-only")).toEqual([]);
  });
});
