import { beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactElement, type ReactNode } from "react";

const { SCOPE, RENDER, STF_STATE } = vi.hoisted(() => {
  const stf = { shadow: 0, midtone: 0.5, highlight: 1 };
  return {
    SCOPE: { imageKey: "C:/data/673nmos.fits", basePath: null },
    RENDER: { chain: { steps: {}, psfKernel: null, base: null }, processed: null },
    STF_STATE: { compositeStfR: stf, compositeStfG: stf, compositeStfB: stf, compositeStfLinked: true },
  };
});

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../hooks/useHistory", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../hooks/useHistory")>()),
  useHistoryScope: () => SCOPE,
}));
vi.mock("../../../context/PreviewContext", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../context/PreviewContext")>()),
  useRenderContext: () => RENDER,
}));
vi.mock("../../../context/CompositeContext", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../context/CompositeContext")>()),
  useCompositeStf: () => STF_STATE,
}));
vi.mock("../../../hooks/useProgress", () => ({
  useProgress: () => ({ current: 0, total: 0, percent: 0, stage: "", active: false, reset: () => {} }),
}));

import { hooks } from "../../../test/hookRuntime";
import { RunButton } from "../../ui";
import { historyRerunStore } from "../../../utils/historyRerunStore";
import ArcsinhStretchPanel from "../ArcsinhStretchPanel";
import DeconvolutionPanel from "../DeconvolutionPanel";
import type { ProcessedFile } from "../../../shared/types";

type AnyProps = Record<string, unknown> & { children?: ReactNode };

const RAW = "C:/data/673nmos.fits";

const FILE: ProcessedFile = {
  id: "run-note",
  name: "673nmos.fits",
  path: RAW,
  sourcePath: RAW,
  imageRef: null,
  size: 1,
  status: "done",
  result: { png_path: "C:/out/673nmos.png", previewUrl: "asset://C:/out/673nmos.png", dimensions: [64, 64], elapsed_ms: 1, is_rgb: false },
  error: null,
  startedAt: null,
  finishedAt: null,
};

const BASE_PROPS = {
  selectedFile: FILE,
  outputDir: "C:/out",
  fileKey: `${FILE.id}|${RAW}`,
  compositeMode: false,
  compositeInput: null,
  onCompositeDone: () => {},
  fileName: FILE.name,
  disabledReason: null as string | null,
};

type PanelProps = typeof BASE_PROPS & { disabledReasonId?: string; runNoteId?: string; onRunLabelChange?: (label: string) => void };

const stretch = ArcsinhStretchPanel as unknown as (props: PanelProps) => ReactNode;
const deconv = DeconvolutionPanel as unknown as (props: PanelProps) => ReactNode;

function elements(node: ReactNode): ReactElement<AnyProps>[] {
  if (Array.isArray(node)) return node.flatMap(elements);
  if (!isValidElement<AnyProps>(node)) return [];
  return [node, ...elements(node.props.children)];
}

function runButton(tree: ReactNode): ReactElement<AnyProps> {
  const run = elements(tree).find((el) => el.type === RunButton);
  if (!run) throw new Error("no RunButton in the panel");
  return run;
}

function methodButton(tree: ReactNode, method: string): ReactElement<AnyProps> {
  const button = elements(tree).find((el) => el.type === "button" && el.props["data-stretch-method"] === method);
  if (!button) throw new Error(`no ${method} method button`);
  return button;
}

function render(panel: (props: PanelProps) => ReactNode, props: Partial<PanelProps> = {}): ReactNode {
  return hooks.render(panel, { ...BASE_PROPS, ...props }) as ReactNode;
}

beforeEach(() => {
  hooks.mount();
  historyRerunStore.clear();
});

describe("Stretch reports the tool its Run will record", () => {
  it("reports Arcsinh Stretch, then GHS Stretch once GHS is chosen", () => {
    const onRunLabelChange = vi.fn();
    const tree = render(stretch, { onRunLabelChange });
    expect(onRunLabelChange).toHaveBeenLastCalledWith("Arcsinh Stretch");
    (methodButton(tree, "ghs").props.onClick as () => void)();
    render(stretch, { onRunLabelChange });
    expect(onRunLabelChange).toHaveBeenLastCalledWith("GHS Stretch");
    (methodButton(render(stretch, { onRunLabelChange }), "arcsinh").props.onClick as () => void)();
    render(stretch, { onRunLabelChange });
    expect(onRunLabelChange).toHaveBeenLastCalledWith("Arcsinh Stretch");
  });
});

describe.each([
  ["Stretch", stretch],
  ["Deconvolution", deconv],
])("%s Run points at the replaced-steps note", (_name, panel) => {
  it("joins the RGB reason and the replaced-steps note, and keeps the note alone without a reason", () => {
    expect(runButton(render(panel, { disabledReason: "RGB FITS reason", disabledReasonId: "rgb-note", runNoteId: "replaced-note" })).props.describedBy).toBe(
      "rgb-note replaced-note",
    );
    expect(runButton(render(panel, { runNoteId: "replaced-note" })).props.describedBy).toBe("replaced-note");
    expect(runButton(render(panel, { disabledReason: "RGB FITS reason", disabledReasonId: "rgb-note" })).props.describedBy).toBe("rgb-note");
    expect(runButton(render(panel)).props.describedBy).toBeUndefined();
  });
});
