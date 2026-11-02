import { isValidElement, type ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SaguiState } from "../saguiStore";

const mocks = vi.hoisted(() => ({ open: vi.fn(), restore: vi.fn() }));
const state: SaguiState = { config: { input_path: "", output_dir: "", rscript_path: "", library_path: "", n_regions: 20, starlet: true, starlet_scales: [1, 2, 3], starlet_threshold: 3, dq_exclude_bits: 4294967295, psf_target: "", mode: "ward", knn_k: 40 }, input: null, runtime: null, status: "idle", operation: null, installation: null, installParent: "", progress: null, result: null, outputDir: null, unit: null, error: null };
vi.mock("react", async (original) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await original()));
vi.mock("../saguiStore", async (original) => ({ ...await original<typeof import("../saguiStore")>(), useSaguiState: () => state, saguiStore: { get: () => state, restore: mocks.restore } }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: mocks.open, save: vi.fn() }));
vi.mock("../../../hooks/useFileStore", () => ({ fileStore: { getSelectedFile: () => null } }));
vi.mock("../../../hooks/useFileIngest", () => ({ showWhenLoaded: vi.fn() }));
import { hooks } from "../../../test/hookRuntime";
import SaguiPanel from "../SaguiPanel";

type Props = { children?: ReactNode; onClick?: () => void; label?: string; value?: string };
function find(node: ReactNode, match: (props: Props) => boolean): Props | undefined {
  if (Array.isArray(node)) return node.map((child) => find(child, match)).find(Boolean);
  if (isValidElement<Props>(node)) return match(node.props) ? node.props : find(node.props.children, match);
  return undefined;
}

beforeEach(() => { hooks.mount(); vi.clearAllMocks(); state.status = "idle"; state.config = { ...state.config, starlet_scales: [1, 2, 3] }; });
afterEach(() => hooks.mount());

describe("SAGUI saved-result chooser", () => {
  it("loads the chosen manifest and synchronizes the editable starlet scales", async () => {
    mocks.open.mockResolvedValueOnce("/old/job/manifest.json");
    mocks.restore.mockImplementationOnce(async () => { state.status = "restored"; state.config = { ...state.config, starlet_scales: [2, 4] }; });
    const tree = hooks.render(SaguiPanel, {}) as ReactNode;
    find(tree, (props) => props.children === "Open previous SAGUI result")!.onClick!();
    await vi.waitFor(() => expect(mocks.restore).toHaveBeenCalledWith("/old/job/manifest.json"));
    expect(mocks.open).toHaveBeenCalledWith({ title: "Open previous SAGUI result", multiple: false, filters: [{ name: "SAGUI manifest", extensions: ["json"] }] });
    const updated = hooks.render(SaguiPanel, {}) as ReactNode;
    const scales = find(updated, (props) => props.label === "Starlet scales (1–10)")!.children;
    expect(isValidElement<Props>(scales) && scales.props.value).toBe("2, 4");
  });

  it("leaves the current selection unchanged when the chooser is cancelled", async () => {
    mocks.open.mockResolvedValueOnce(null);
    const config = state.config;
    find(hooks.render(SaguiPanel, {}) as ReactNode, (props) => props.children === "Open previous SAGUI result")!.onClick!();
    await Promise.resolve();
    expect(mocks.restore).not.toHaveBeenCalled();
    expect(state.config).toBe(config);
  });
});
