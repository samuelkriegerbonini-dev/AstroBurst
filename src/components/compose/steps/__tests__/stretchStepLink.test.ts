import { describe, expect, it, vi, beforeEach } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

const { captured, context, restretchMock, setLinkedMock, setStfMock } = vi.hoisted(() => ({
  captured: {
    toggles: new Map<string, { checked: boolean; onChange: (v: boolean) => void }>(),
    runs: new Map<string, () => void>(),
  },
  context: {
    linked: false,
    stf: {
      r: { shadow: 0.01, midtone: 0.2, highlight: 1 },
      g: { shadow: 0.02, midtone: 0.3, highlight: 1 },
      b: { shadow: 0.03, midtone: 0.4, highlight: 1 },
    },
    autoStf: {
      r: { shadow: 0.004, midtone: 0.12, highlight: 1 },
      g: { shadow: 0.005, midtone: 0.25, highlight: 1 },
      b: { shadow: 0.006, midtone: 0.35, highlight: 1 },
    },
  },
  restretchMock: vi.fn(),
  setLinkedMock: vi.fn(),
  setStfMock: vi.fn(),
}));

vi.mock("../../../ui", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../../ui")>();
  return {
    ...actual,
    Toggle: (props: { label: string; checked: boolean; onChange: (v: boolean) => void }) => {
      captured.toggles.set(props.label, { checked: props.checked, onChange: props.onChange });
      return null;
    },
    RunButton: (props: { label: string; onClick: () => void }) => {
      captured.runs.set(props.label, props.onClick);
      return null;
    },
  };
});

vi.mock("../../../../context/CompositeContext", () => ({
  useCompositeStf: () => ({
    compositeStfR: context.stf.r,
    compositeStfG: context.stf.g,
    compositeStfB: context.stf.b,
    compositeStfLinked: context.linked,
    compositeAutoStfR: context.autoStf.r,
    compositeAutoStfG: context.autoStf.g,
    compositeAutoStfB: context.autoStf.b,
  }),
  useCompositeActions: () => ({ setCompositeStfLinked: setLinkedMock, setCompositeStf: setStfMock }),
}));

vi.mock("../../../../context/PreviewContext", () => ({
  useRenderActions: () => ({ currentFileKey: () => null, publishProcessed: vi.fn() }),
}));

vi.mock("../../../../services/compose", () => ({
  restretchComposite: restretchMock,
  renderLinearCompositePreview: vi.fn(),
}));

vi.mock("../../../../infrastructure/tauri", () => ({
  typedInvoke: vi.fn(),
  withPreview: vi.fn(),
  getOutputDir: vi.fn(async () => "/out"),
  getPreviewUrl: vi.fn(async (p: string) => `asset://${p}`),
}));

import StretchStep from "../StretchStep";
import { INITIAL_STATE } from "../../../../utils/wizard";

const AUTO_STF_STATE = { ...INITIAL_STATE, stretchMode: "auto_stf" as const, compositeReady: true };

function render(): void {
  captured.toggles.clear();
  captured.runs.clear();
  renderToStaticMarkup(createElement(StretchStep, {
    state: AUTO_STF_STATE,
    onStretchChange: () => {},
    onMaskParams: () => {},
    onMask: () => {},
    onResult: () => {},
    onChannelOutput: () => {},
    onCompositeOp: () => {},
  }));
}

function linkToggle() {
  const toggle = captured.toggles.get("Link channels");
  if (!toggle) throw new Error("Link channels toggle not rendered");
  return toggle;
}

async function runRestretch(): Promise<unknown[]> {
  const run = captured.runs.get("Re-stretch Composite");
  if (!run) throw new Error("run button not rendered");
  run();
  await vi.waitFor(() => expect(restretchMock).toHaveBeenCalledTimes(1));
  return restretchMock.mock.calls[0];
}

describe("wizard Stretch Link channels", () => {
  beforeEach(() => {
    restretchMock.mockReset();
    restretchMock.mockResolvedValue({ png_path: "/out/rgb_composite_1.png", elapsed_ms: 3 });
    setLinkedMock.mockReset();
    setStfMock.mockReset();
  });

  it.each([false, true])("reads the composite linked flag %s", (linked) => {
    context.linked = linked;
    render();
    expect(linkToggle().checked).toBe(linked);
  });

  it("writes the toggle to the composite context", () => {
    context.linked = false;
    render();
    linkToggle().onChange(true);
    expect(setLinkedMock).toHaveBeenCalledWith(true);
  });

  it("turning Link channels on also equalises the shared STF on the R channel, as the GPU STF panel does", () => {
    context.linked = false;
    render();
    linkToggle().onChange(true);
    expect(setStfMock).toHaveBeenCalledTimes(1);
    expect(setStfMock).toHaveBeenCalledWith(context.stf.r, context.stf.r, context.stf.r);
  });

  it("turning Link channels off keeps the shared STF", () => {
    context.linked = true;
    render();
    linkToggle().onChange(false);
    expect(setLinkedMock).toHaveBeenCalledWith(false);
    expect(setStfMock).not.toHaveBeenCalled();
  });

  it("runs Auto STF unlinked with the three per-channel STFs", async () => {
    context.linked = false;
    render();
    const args = await runRestretch();
    expect(args.slice(1, 4)).toEqual([context.autoStf.r, context.autoStf.g, context.autoStf.b]);
    expect(args[6]).toBe(false);
  });

  it("runs Auto STF linked with the R STF on all three channels even when the wizard's G and B differ", async () => {
    context.linked = true;
    render();
    const args = await runRestretch();
    expect(args.slice(1, 4)).toEqual([context.autoStf.r, context.autoStf.r, context.autoStf.r]);
    expect(args[6]).toBe(true);
  });
});
