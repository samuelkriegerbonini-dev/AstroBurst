import { isValidElement, type ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CubePrepReport } from "../../../shared/types/cubeprep";
import type { CubePrepState } from "../cubePrepStore";
import { createCubePrepDraft } from "../cubePrepModel";

const mocks = vi.hoisted(() => ({ prepare: vi.fn(), dispatch: vi.fn() }));
const report: CubePrepReport = { recipe_version: 1, name: "field", output_dir: "/out", cube: "final.fits", steps: [], files: [], warnings: [], elapsed_ms: 1 };
const state: CubePrepState = { draft: createCubePrepDraft(), status: "complete", validation: null, progress: null, report, outputDir: "/out", error: null };
vi.mock("react", async (original) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await original()));
vi.mock("../cubePrepStore", () => ({ useCubePrepState: () => state, cubePrepStore: { get: () => state } }));
vi.mock("../../sagui/saguiStore", () => ({ saguiStore: { prepareCubePrep: mocks.prepare } }));
vi.mock("../../../hooks/useDockLayout", () => ({ dockStore: { dispatch: mocks.dispatch } }));
vi.mock("../../../hooks/useFileStore", () => ({ fileStore: { getFiles: () => [] } }));
vi.mock("../../../hooks/useFileIngest", () => ({ showWhenLoaded: vi.fn() }));
vi.mock("../../../services/cubeprep", () => ({ cubePrepFilters: async () => ({ datasets: [], filters: [] }) }));
import { hooks } from "../../../test/hookRuntime";
import CubePrepPanel from "../CubePrepPanel";

function action(node: ReactNode): (() => void) | undefined {
  if (Array.isArray(node)) return node.map(action).find(Boolean);
  if (isValidElement<{ children?: ReactNode; onClick?: () => void }>(node)) return node.props.children === "Send to SAGUI" ? node.props.onClick : action(node.props.children);
  return undefined;
}

beforeEach(() => { hooks.mount(); vi.clearAllMocks(); state.report = report; });
afterEach(() => hooks.mount());

describe("Cube Prep to SAGUI desktop action", () => {
  it("opens the existing dock tool only after successful input preparation", async () => {
    let resolve!: (accepted: boolean) => void;
    mocks.prepare.mockReturnValueOnce(new Promise<boolean>((yes) => { resolve = yes; }));
    const click = action(hooks.render(CubePrepPanel, {}) as ReactNode);
    expect(click).toBeDefined();
    click!();
    expect(mocks.prepare).toHaveBeenCalledWith(report, expect.any(Function));
    expect(mocks.dispatch).not.toHaveBeenCalled();
    resolve(true);
    await vi.waitFor(() => expect(mocks.dispatch).toHaveBeenCalledWith({ type: "open", tool: "sagui" }));
  });

  it("makes changed reports and an unmounted sender obsolete without opening the dock", async () => {
    mocks.prepare.mockResolvedValueOnce(false);
    action(hooks.render(CubePrepPanel, {}) as ReactNode)!();
    const current = mocks.prepare.mock.calls[0][1] as () => boolean;
    expect(current()).toBe(true);
    state.report = null;
    expect(current()).toBe(false);
    state.report = report;
    hooks.mount();
    expect(current()).toBe(false);
    await Promise.resolve();
    expect(mocks.dispatch).not.toHaveBeenCalled();
  });
});
