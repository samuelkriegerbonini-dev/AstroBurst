import { isValidElement, type ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SaguiState } from "../saguiStore";
import type { CubePrepReport } from "../../../shared/types/cubeprep";

const mocks = vi.hoisted(() => ({ open: vi.fn(), restore: vi.fn(), openUrl: vi.fn(), inspect: vi.fn(), inspectRuntime: vi.fn(), prepare: vi.fn(), edit: vi.fn(), reveal: vi.fn(), report: null as CubePrepReport | null }));
const state: SaguiState = { config: { input_path: "", output_dir: "", rscript_path: "", library_path: "", n_regions: 20, starlet: true, starlet_scales: [1, 2, 3], starlet_threshold: 3, dq_exclude_bits: 4294967295, psf_target: "", mode: "ward", knn_k: 40 }, input: null, runtime: null, status: "idle", operation: null, installation: null, installParent: "", progress: null, result: null, outputDir: null, unit: null, error: null };
vi.mock("react", async (original) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await original()));
vi.mock("../saguiStore", async (original) => ({ ...await original<typeof import("../saguiStore")>(), useSaguiState: () => state, saguiStore: { get: () => state, restore: mocks.restore, inspect: mocks.inspect, inspectRuntime: mocks.inspectRuntime, prepareCubePrep: mocks.prepare, edit: mocks.edit, isJobActive: () => false } }));
vi.mock("../../cubeprep/cubePrepStore", () => ({ cubePrepStore: { get: () => ({ report: mocks.report }) } }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: mocks.open, save: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: mocks.openUrl, revealItemInDir: vi.fn() }));
vi.mock("../../../utils/revealInFolder", () => ({ revealInFolder: mocks.reveal }));
vi.mock("../../../hooks/useFileStore", () => ({ fileStore: { getSelectedFile: () => null } }));
vi.mock("../../../hooks/useFileIngest", () => ({ showWhenLoaded: vi.fn() }));
import { hooks } from "../../../test/hookRuntime";
import SaguiPanel from "../SaguiPanel";
import { PathNote } from "../../cubeprep/CubePrepPanel";

type Props = { children?: ReactNode; onClick?: () => void; onChange?: (event: { target: { value: string } }) => void; label?: string; value?: string; role?: string };
function find(node: ReactNode, match: (props: Props) => boolean): Props | undefined {
  if (Array.isArray(node)) return node.map((child) => find(child, match)).find(Boolean);
  if (isValidElement<Props>(node)) return match(node.props) ? node.props : find(node.props.children, match);
  return undefined;
}
function texts(node: ReactNode): string[] {
  if (typeof node === "string") return [node];
  if (Array.isArray(node)) return node.flatMap(texts);
  return isValidElement<Props>(node) ? texts(node.props.children) : [];
}
function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => { resolve = done; });
  return { promise, resolve };
}
const panel = () => hooks.render(SaguiPanel, {}) as ReactNode;
const tick = () => new Promise((done) => setTimeout(done, 0));

beforeEach(() => { hooks.mount(); vi.clearAllMocks(); mocks.report = null; state.status = "idle"; state.operation = null; state.config = { ...state.config, starlet_scales: [1, 2, 3] }; });
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

describe("SAGUI R requirement", () => {
  it("opens the CRAN archive folder of the pinned R version outside the app", async () => {
    find(hooks.render(SaguiPanel, {}) as ReactNode, (props) => props.children === "CRAN archive")!.onClick!();
    await vi.waitFor(() => expect(mocks.openUrl).toHaveBeenCalledWith("https://cran.r-project.org/bin/windows/base/old/4.6.2/"));
  });
});

describe("SAGUI check status (SCIENCE-18)", () => {
  it.each([
    ["Check R installation", () => mocks.inspectRuntime, "Checking the R installation…"],
    ["Check cube and runtime", () => mocks.inspect, "Checking cube and official runtime…"],
    ["Use Cube Prep result", () => mocks.prepare, "Checking the prepared cube…"],
  ])("announces %s by what it inspects and stops claiming it once that check ends", async (label, action, copy) => {
    mocks.report = { recipe_version: 1, name: "field", output_dir: "/out", cube: "cube.fits", steps: [], files: [], warnings: [], elapsed_ms: 1 };
    const pending = deferred();
    action().mockReturnValueOnce(pending.promise);
    find(panel(), (props) => props.children === label)!.onClick!();
    expect(action()).toHaveBeenCalledOnce();
    state.status = "checking";
    expect(texts(panel())).toContain(copy);
    pending.resolve();
    await tick();
    const after = texts(panel());
    expect(after).not.toContain(copy);
    expect(after).toContain("Checking…");
  });
});

describe("SAGUI advanced clustering", () => {
  it("sends the chosen SED scaling with the rest of the configuration", () => {
    const select = find(panel(), (props) => props.label === "SED scaling")!.children;
    expect(isValidElement<Props>(select) && select.props.value).toBe("median_centre");
    (isValidElement<Props>(select) ? select.props.onChange : undefined)!({ target: { value: "median_mad" } });
    expect(mocks.edit).toHaveBeenCalledWith({ ...state.config, sed_scaling: "median_mad" });
  });
});

describe("Run folder and product notes (SCIENCE-27)", () => {
  it("shows a failed Show in folder beside its own path, not in the panel error", async () => {
    mocks.reveal.mockResolvedValueOnce("Could not open the folder C:\\work: access denied");
    const note = (path: string) => hooks.render(PathNote, { label: "Run folder", path }) as ReactNode;
    find(note("C:\\work\\sagui-1"), (props) => props.children === "Show in folder")!.onClick!();
    expect(mocks.reveal).toHaveBeenCalledWith("C:\\work\\sagui-1");
    await tick();
    expect(find(note("C:\\work\\sagui-1"), (props) => props.role === "alert")?.children).toBe("Could not open the folder C:\\work: access denied");
    expect(find(note("C:\\work\\sagui-2"), (props) => props.role === "alert")).toBeUndefined();
  });
});
