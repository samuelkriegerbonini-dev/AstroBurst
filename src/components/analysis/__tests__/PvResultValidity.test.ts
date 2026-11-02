import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactNode } from "react";

const backend = vi.hoisted(() => ({ run: vi.fn(), publish: vi.fn(), log: vi.fn() }));

vi.mock("react", async (original) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await original()));
vi.mock("../../../services/pv", () => ({ computePvDiagram: backend.run }));
vi.mock("../../../infrastructure/tauri", () => ({ getOutputDir: async () => "C:/out" }));
vi.mock("../../../context/PreviewContext", () => ({
  useRenderActions: () => ({ publishProcessed: backend.publish, resetProcessed: () => {} }),
  useRenderContext: () => ({ processed: null }),
}));
vi.mock("../../../hooks/useRegionKey", () => ({ useRegionKey: () => "cube.fits" }));
vi.mock("../../../hooks/useRegionStore", () => ({
  useRegionDoc: () => ({
    selectedId: "line",
    regions: [{ id: "line", shape: { shape: "line", x1: 1, y1: 1, x2: 4, y2: 1 }, props: { include: true }, backgroundId: null }],
  }),
}));
vi.mock("../../../utils/measurementLog", () => ({ measurementLog: { append: backend.log }, pvEntry: (value: unknown) => value }));

import { hooks } from "../../../test/hookRuntime";
import PvPanel from "../PvPanel";

type Props = Record<string, unknown>;

function runButton(node: ReactNode): Props | null {
  if (Array.isArray(node)) {
    for (const child of node) {
      const found = runButton(child);
      if (found) return found;
    }
  } else if (isValidElement<Props>(node)) {
    if (node.type === "button" && Array.isArray(node.props.children) && node.props.children.includes("Run PV")) return node.props;
    return runButton(node.props.children as ReactNode);
  }
  return null;
}

function run(): Promise<void> {
  const props = { filePath: "cube.fits", fileKey: "file|cube.fits", inputRevision: "revision-1", cubeDims: { frames: 4 } };
  const panel = (PvPanel as unknown as { type: (props: Props) => ReactNode }).type;
  hooks.render(panel, props);
  const tree = hooks.render(panel, props) as ReactNode;
  const button = runButton(tree);
  if (!button) throw new Error("Run PV control missing");
  return (button.onClick as () => Promise<void>)();
}

const result = { fits_path: "pv.fits", previewUrl: "pv.png", dimensions: [4, 4], z0: 0, z1: 3 };

beforeEach(() => {
  hooks.mount();
  backend.run.mockReset().mockResolvedValue(result);
  backend.publish.mockReset();
  backend.log.mockReset();
});

afterEach(() => hooks.mount());

describe("PV panel request lifetime", () => {
  it("publishes a completed result with its input revision while mounted", async () => {
    await run();
    expect(backend.publish).toHaveBeenCalledWith("file|cube.fits", expect.objectContaining({ fitsPath: "pv.fits", inputRevision: "revision-1" }));
    expect(backend.log).toHaveBeenCalledTimes(1);
  });

  it("does not publish or log a result returned after unmount", async () => {
    let resolve!: (value: typeof result) => void;
    backend.run.mockReturnValue(new Promise((yes) => { resolve = yes; }));
    const pending = run();
    await vi.waitFor(() => expect(backend.run).toHaveBeenCalledTimes(1));
    hooks.mount();
    resolve(result);
    await pending;
    expect(backend.publish).not.toHaveBeenCalled();
    expect(backend.log).not.toHaveBeenCalled();
  });
});
