import { isValidElement, type ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SaguiState } from "../saguiStore";
import type { CubePrepReport } from "../../../shared/types/cubeprep";
import type { SaguiInputInfo, SaguiResult } from "../../../shared/types/sagui";
import type { CubePrepDraft } from "../../cubeprep/cubePrepModel";

const mocks = vi.hoisted(() => ({ open: vi.fn(), save: vi.fn(), read: vi.fn(), write: vi.fn(), prepare: vi.fn(), edit: vi.fn(), inspectCube: vi.fn(), dispatch: vi.fn(), prepEdit: vi.fn(), selected: null as { path: string } | null, report: null as CubePrepReport | null, draft: null as CubePrepDraft | null }));
const CONFIG = { input_path: "", output_dir: "", rscript_path: "", library_path: "", n_regions: 20, starlet: true, starlet_scales: [1, 2, 3], starlet_threshold: 3, dq_exclude_bits: 1, psf_target: "", mode: "ward" as const, knn_k: 40 };
const state: SaguiState = { config: { ...CONFIG }, input: null, runtime: null, status: "idle", operation: null, installation: null, installParent: "", progress: null, result: null, outputDir: null, error: null };
vi.mock("react", async (original) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await original()));
vi.mock("../saguiStore", async (original) => ({ ...await original<typeof import("../saguiStore")>(), useSaguiState: () => state, saguiStore: { get: () => state, edit: mocks.edit, inspectCube: mocks.inspectCube, prepareCubePrep: mocks.prepare, inspect: vi.fn(), inspectRuntime: vi.fn(), restore: vi.fn(), run: vi.fn(), cancel: vi.fn(), install: vi.fn(), setInstallParent: vi.fn(), isJobActive: () => false, canCancel: () => false } }));
vi.mock("../../cubeprep/cubePrepStore", () => ({ cubePrepStore: { get: () => ({ report: mocks.report, draft: mocks.draft }), edit: mocks.prepEdit, isJobActive: () => false } }));
vi.mock("../../../hooks/useDockLayout", () => ({ dockStore: { dispatch: mocks.dispatch } }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: mocks.open, save: mocks.save }));
vi.mock("@tauri-apps/plugin-fs", () => ({ readTextFile: mocks.read, writeTextFile: mocks.write }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn(), revealItemInDir: vi.fn() }));
vi.mock("../../../hooks/useFileStore", () => ({ fileStore: { getSelectedFile: () => mocks.selected } }));
vi.mock("../../../hooks/useFileIngest", () => ({ showWhenLoaded: vi.fn() }));
vi.mock("../../../hooks/useResolutionSettings", async () => {
  const { DEFAULT_RESOLUTION_SETTINGS } = await import("../../../utils/resolutionText");
  return { useResolutionSettings: () => DEFAULT_RESOLUTION_SETTINGS };
});
import { hooks } from "../../../test/hookRuntime";
import { createCubePrepDraft } from "../../cubeprep/cubePrepModel";
import SaguiPanel from "../SaguiPanel";

type Props = Record<string, unknown> & { children?: ReactNode; onClick?: () => void; onChange?: (event: { target: { value: string; checked?: boolean } }) => void; onBlur?: () => void };
function findAll(node: ReactNode, match: (props: Props, type: unknown) => boolean, found: Props[] = []): Props[] {
  if (Array.isArray(node)) { for (const child of node) findAll(child, match, found); return found; }
  if (!isValidElement<Props>(node)) return found;
  if (match(node.props, node.type)) found.push(node.props);
  findAll(node.props.children, match, found);
  return found;
}
const find = (node: ReactNode, match: (props: Props, type: unknown) => boolean) => findAll(node, match)[0];
function texts(node: ReactNode): string[] {
  if (typeof node === "string" || typeof node === "number") return [String(node)];
  if (Array.isArray(node)) return node.flatMap(texts);
  return isValidElement<Props>(node) ? texts(node.props.children) : [];
}
const text = (node: ReactNode) => texts(node).join("");
const panel = () => hooks.render(SaguiPanel, {}) as ReactNode;
const button = (tree: ReactNode, label: string) => find(tree, (p, type) => type === "button" && text(p.children) === label);
const field = (tree: ReactNode, label: string) => {
  const holder = find(tree, (p) => p.label === label);
  return isValidElement<Props>(holder?.children) ? holder.children.props : undefined;
};
const REPORT: CubePrepReport = { recipe_version: 1, name: "field", output_dir: "C:\\runs\\cubeprep-field-1", cube: "09_export\\prepared_cube.fits", steps: [], files: [], warnings: [], elapsed_ms: 1 };
const PREPARED = "C:\\runs\\cubeprep-field-1\\09_export\\prepared_cube.fits";
const INPUT: SaguiInputInfo = { width: 43, height: 43, bands: [{ filter: "F814W", pivot_um: 0.8046 }, { filter: "F150W", pivot_um: 1.5007 }, { filter: "F444W", pivot_um: 4.4043 }], unit: "Jy/pixel", has_err: true, has_dq: true, dq_flags: [{ filter: "F814W", table: "hst", flagged_finite: { COSMIC_RAY: 12, REED_SOLOMON: 2 } }, { filter: "F150W", table: "jwst", flagged_finite: { DO_NOT_USE: 5 } }, { filter: "F444W", table: "jwst", flagged_finite: {} }], estimated_memory_bytes: 1, warnings: [] };
const tick = () => new Promise((done) => setTimeout(done, 0));

beforeEach(() => {
  hooks.mount();
  vi.clearAllMocks();
  Object.assign(state, { config: { ...CONFIG }, input: null, runtime: null, status: "idle", operation: null, result: null, outputDir: null, error: null, progress: null });
  Object.assign(mocks, { selected: null, report: null, draft: null });
  mocks.edit.mockImplementation((config: SaguiState["config"]) => { state.config = config; });
});
afterEach(() => hooks.mount());

describe("SAGUI cube selection", () => {
  it("hands the Cube Prep cube over with its verified target when it is chosen as the selected FITS (PREPUI-10)", async () => {
    mocks.report = REPORT;
    mocks.selected = { path: PREPARED };
    mocks.prepare.mockResolvedValueOnce(true);
    button(panel(), "Use selected FITS")!.onClick!();
    await vi.waitFor(() => expect(mocks.prepare).toHaveBeenCalledWith(REPORT, expect.any(Function)));
    expect(mocks.edit).not.toHaveBeenCalled();
  });

  it("checks every other chosen cube at once, without starting R (proposal 14)", async () => {
    mocks.selected = { path: "C:\\data\\external.fits" };
    button(panel(), "Use selected FITS")!.onClick!();
    expect(mocks.edit).toHaveBeenCalledWith(expect.objectContaining({ input_path: "C:\\data\\external.fits", output_dir: "C:\\data" }));
    await vi.waitFor(() => expect(mocks.inspectCube).toHaveBeenCalledOnce());
    mocks.open.mockResolvedValueOnce("C:\\data\\other.fits");
    find(panel(), (p) => p["aria-label"] === "Browse prepared multiband FITS cube")!.onClick!();
    await vi.waitFor(() => expect(mocks.inspectCube).toHaveBeenCalledTimes(2));
    expect(state.config.input_path).toBe("C:\\data\\other.fits");
    field(panel(), "Prepared multiband FITS cube")!.onBlur!();
    await tick();
    expect(mocks.inspectCube).toHaveBeenCalledTimes(3);
  });

  it("asks before a recipe from SAGUI replaces a Cube Prep draft that has inputs (PREPUI-22)", async () => {
    mocks.draft = { ...createCubePrepDraft(), recipe: { ...createCubePrepDraft().recipe, inputs: [{ path: "a.fits", dataset: "JWST", filter: "F200W" }] } };
    state.config = { ...CONFIG, input_path: "C:\\recipes\\d1.json" };
    mocks.read.mockResolvedValue(JSON.stringify({ recipe_version: 1, inputs: [], steps: [] }));
    button(panel(), "Open this recipe in Cube Prep")!.onClick!();
    await tick();
    expect(mocks.prepEdit).not.toHaveBeenCalled();
    const asking = panel();
    expect(text(asking)).toContain("Cube Prep holds a draft with 1 input; opening this recipe replaces it.");
    button(asking, "Keep the draft")!.onClick!();
    expect(button(panel(), "Open this recipe in Cube Prep")).toBeDefined();
    button(panel(), "Open this recipe in Cube Prep")!.onClick!();
    button(panel(), "Replace the draft")!.onClick!();
    await vi.waitFor(() => expect(mocks.prepEdit).toHaveBeenCalledOnce());
    expect(mocks.dispatch).toHaveBeenCalledWith({ type: "open", tool: "cubeprep" });
  });

  it("sends an empty window to Cube Prep first (proposal 23)", () => {
    const empty = find(panel(), (p) => "data-empty-cube" in p)!;
    expect(text(empty.children)).toContain("No cube yet.");
    button(empty.children, "Prepare in Cube Prep")!.onClick!();
    expect(mocks.dispatch).toHaveBeenCalledWith({ type: "open", tool: "cubeprep" });
  });
});

describe("SAGUI settings", () => {
  it("offers the bands of the checked cube as PSF target, says when Cube Prep verified it and warns on a name outside the cube (PREPUI-04, proposal 15)", () => {
    mocks.report = { ...REPORT, steps: [] };
    state.input = INPUT;
    state.config = { ...CONFIG, input_path: PREPARED };
    const target = field(panel(), "Verified common PSF target")!;
    expect(texts(target.children)).toEqual(["Choose the band every plane was matched to", "F814W", "F150W", "F444W", "External PSF, not a band of this cube"]);
    target.onChange!({ target: { value: "F444W" } });
    expect(state.config.psf_target).toBe("F444W");
    state.config = { ...state.config, psf_target: "F200W" };
    const external = panel();
    expect(field(external, "Verified common PSF target")!.value).toBe("__external");
    expect(field(external, "External PSF all bands were matched to")!.value).toBe("F200W");
    expect(text(external)).toContain("F200W is not a band of this cube (F814W, F150W, F444W).");
  });

  it("shows the DQ flags of each band, what the mask excludes, and offers the Cube Prep masks for a mixed cube (PREPUI-07, proposal 16)", () => {
    state.input = INPUT;
    const tree = panel();
    const table = find(tree, (p) => "data-dq-flags" in p)!;
    expect(texts(table.children)).toEqual(["Band", "DQ table", "Flagged finite pixels", "F814W", "HST", "REED_SOLOMON 2 (excluded)", ", ", "COSMIC_RAY 12", "F150W", "JWST", "DO_NOT_USE 5 (excluded)", "F444W", "JWST", "none"]);
    expect(text(tree)).toContain("The planes use different DQ tables (HST: F814W; JWST: F150W, F444W) and one mask applies to all of them.");
    expect(field(tree, "DQ bits to exclude")!.value).toBe("1");
    button(tree, "Use 16383")!.onClick!();
    expect(state.config.dq_exclude_bits).toBe(16383);
    field(panel(), "DQ bits to exclude")!.onChange!({ target: { value: "custom" } });
    const custom = field(panel(), "Custom DQ mask (decimal)")!;
    expect(custom.value).toBe(16383);
    custom.onChange!({ target: { value: "7" } });
    expect(state.config.dq_exclude_bits).toBe(7);
  });

  it("says before the run whether exact Ward can take the footprint (proposal 18)", () => {
    state.input = { ...INPUT, width: 500, height: 500 };
    expect(text(panel())).toContain("Exact Ward refuses more than 65,536 support pixels and the full footprint has 250,000.");
  });

  it("captions Run with the first problem, links it to the list and marks the current step (proposals 6, 13, PREPUI-21)", () => {
    const tree = panel();
    const run = find(tree, (p) => "runningLabel" in p)!;
    expect(run.label).toBe("Choose a prepared multiband FITS cube.");
    const list = find(tree, (p, type) => type === "ul" && typeof p.id === "string")!;
    expect(run.describedBy).toBe(list.id);
    const phases = (node: ReactNode) => findAll(node, (p) => "phase" in p && "section" in p).map((p) => [p.number, p.section, p.phase]);
    expect(phases(tree)).toEqual([[1, "cube", "current"], [2, "runtime", "pending"], [3, "regions", "pending"], [4, "run", "pending"]]);
    state.input = INPUT;
    state.runtime = { available: true, ready: true, r_version: "4.6.2", sagui_version: "0.0.0.9000", sagui_commit: "56fd3ea776d7c00e056368498da9412bd58b84bb", issues: [] };
    state.config = { ...CONFIG, input_path: PREPARED, output_dir: "C:\\runs", rscript_path: "Rscript", library_path: "lib" };
    expect(phases(panel())).toEqual([[1, "cube", "done"], [2, "runtime", "done"], [3, "regions", "current"], [4, "run", "pending"]]);
  });

  it("names what Install / repair still needs and the fix of each runtime issue (proposal 17, PREPUI-13)", () => {
    state.runtime = { available: true, ready: false, r_version: "4.5.1", sagui_version: null, sagui_commit: null, issues: ["Expected R 4.6.2; found 4.5.1"] };
    const tree = panel();
    expect(find(tree, (p, type) => type === "button" && text(p.children) === "Install / repair SAGUI library")?.disabled).toBe(true);
    expect(text(tree)).toContain("Choose the Rscript executable first.");
    expect(find(tree, (p) => Array.isArray(p.warnings) && p.warnings.includes("Expected R 4.6.2; found 4.5.1: install R 4.6.2 alongside your current R (the Windows installer is in the CRAN archive) and choose its Rscript."))).toBeDefined();
    expect(findAll(tree, (_p, type) => type === "li").map((p) => texts(p.children)[0]).slice(0, 3)).toEqual(["Choose Rscript 4.6.2.", "Choose the private SAGUI library, or install the pinned SAGUI into a new one.", "Check the installation."]);
  });
});

describe("SAGUI result", () => {
  it("summarises the result first and saves the SED of the selected region (proposal 22)", async () => {
    const result: SaguiResult = { run_id: "one", unit: "Jy", output_dir: "C:\\runs\\sagui-one", labels_path: "labels.fits", mask_path: "mask.fits", table_path: "regions.csv", manifest_path: "manifest.json", width: 2, height: 1, labels: [1, 2], warnings: ["w"], regions: [{ region: 1, n_pixels: 1, bands: [{ filter: "F200W", pivot_um: 2, flux: 0.5, error: 0.1 }] }, { region: 2, n_pixels: 1, bands: [{ filter: "F200W", pivot_um: 2, flux: 0.7, error: null }] }] };
    Object.assign(state, { status: "complete", operation: "segment", result, outputDir: result.output_dir });
    const tree = panel();
    expect(text(find(tree, (p) => "data-result-summary" in p)!.children)).toBe("2 regions, 2 support pixels, 1 warning");
    const viewer = find(tree, (p) => "result" in p && "onSaveRegion" in p)!;
    mocks.save.mockResolvedValueOnce("C:\\out\\region1.csv");
    (viewer.onSaveRegion as (region: SaguiResult["regions"][number]) => void)(result.regions[0]);
    await vi.waitFor(() => expect(mocks.write).toHaveBeenCalledWith("C:\\out\\region1.csv", "# SAGUI region 1, 1 pixels, flux unit Jy\nfilter,pivot_um,flux,error,error_correlated\nF200W,2,0.5,0.1,\n"));
    expect(mocks.save).toHaveBeenCalledWith({ defaultPath: "sagui_region_1_sed.csv", filters: [{ name: "CSV", extensions: ["csv"] }] });
  });
});
