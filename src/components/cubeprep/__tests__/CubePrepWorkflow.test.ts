import { isValidElement, type ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CubePrepFilters, CubePrepReport, CubePrepStep } from "../../../shared/types/cubeprep";
import type { ImagePsfFwhm } from "../../../shared/types/resolution";
import type { CubePrepState } from "../cubePrepStore";
import { createCubePrepDraft, type CubePrepDraft } from "../cubePrepModel";

const mocks = vi.hoisted(() => ({ prepare: vi.fn(), dispatch: vi.fn(), filters: vi.fn(), edit: vi.fn(), validate: vi.fn(), run: vi.fn(), psf: vi.fn(), check: vi.fn() }));
const report: CubePrepReport = { recipe_version: 1, name: "field", output_dir: "/out", cube: "final.fits", steps: [], files: [], warnings: [], elapsed_ms: 1 };
const INPUTS = [{ path: "/data/f200w.fits", dataset: "JWST", filter: "F200W" }, { path: "/data/f444w.fits", dataset: "JWST", filter: "F444W" }];
function workflowDraft(steps: CubePrepStep[], profile: CubePrepDraft["profile"] = "scientific"): CubePrepDraft {
  return { ...createCubePrepDraft(), profile, baseDir: "/data", outputParent: "/runs", recipe: { recipe_version: 1, name: "field", inputs: INPUTS, steps } };
}
const state: CubePrepState = { draft: workflowDraft([]), status: "idle", validation: null, progress: null, report: null, outputDir: null, error: null, startedAt: null };
vi.mock("react", async (original) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await original()));
vi.mock("../cubePrepStore", () => ({ useCubePrepState: () => state, cubePrepStore: { get: () => state, edit: mocks.edit, validate: mocks.validate, run: mocks.run, cancel: vi.fn(), isJobActive: () => false } }));
vi.mock("../../sagui/saguiStore", () => ({ saguiStore: { prepareCubePrep: mocks.prepare } }));
vi.mock("../../../hooks/useDockLayout", () => ({ dockStore: { dispatch: mocks.dispatch } }));
vi.mock("../../../hooks/useFileStore", () => ({ fileStore: { getFiles: () => [], getSelectedFile: () => null } }));
vi.mock("../../../hooks/useFileIngest", () => ({ showWhenLoaded: vi.fn() }));
vi.mock("../../../services/cubeprep", () => ({ cubePrepFilters: mocks.filters }));
vi.mock("../../../services/resolution", () => ({ imagePsfFwhm: mocks.psf, resolutionCheck: mocks.check }));
vi.mock("../../../hooks/useResolutionSettings", async () => {
  const { DEFAULT_RESOLUTION_SETTINGS } = await import("../../../utils/resolutionText");
  return { useResolutionSettings: () => DEFAULT_RESOLUTION_SETTINGS };
});
import { hooks } from "../../../test/hookRuntime";
import CubePrepPanel from "../CubePrepPanel";

type Props = Record<string, unknown> & { children?: ReactNode; onClick?: () => void; onChange?: (event: { target: { value: string; checked?: boolean } }) => void };
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
  if (!isValidElement<Props>(node)) return [];
  return texts(node.props.children);
}
const text = (node: ReactNode) => texts(node).join("");
const render = () => hooks.render(CubePrepPanel, {}) as ReactNode;
const tabButton = (tree: ReactNode, label: string) => find(tree, (p) => "aria-pressed" in p && text(p.children).replace(/^\d+\.\s*/, "").replace(/\s*,.*$/, "").startsWith(label));
const openTab = (label: string) => { tabButton(render(), label)!.onClick!(); return render(); };
const editors = (tree: ReactNode) => findAll(tree, (p) => "step" in p && "recipe" in p && "browse" in p);
const button = (tree: ReactNode, label: string) => find(tree, (p, type) => type === "button" && text(p.children) === label);
const field = (tree: ReactNode, label: string) => {
  const holder = find(tree, (p) => p.label === label);
  return isValidElement<Props>(holder?.children) ? holder.children.props : undefined;
};
const lastDraft = () => mocks.edit.mock.calls.at(-1)?.[0] as CubePrepDraft;
const tick = () => new Promise((done) => setTimeout(done, 0));
const REGISTRY: CubePrepFilters = { datasets: ["HST", "JWST"], filters: [["F200W", 19886], ["F410M", 40820], ["F444W", 44043]].map(([filter, pivot_angstrom]) => ({ dataset: "JWST", filter: String(filter), instrument: "NIRCam", group: "wide", pivot_angstrom: Number(pivot_angstrom), aliases: [] })) };

beforeEach(() => {
  hooks.mount();
  vi.clearAllMocks();
  mocks.filters.mockReset().mockResolvedValue({ datasets: [], filters: [] });
  mocks.edit.mockImplementation((draft: CubePrepDraft) => { state.draft = draft; });
  mocks.check.mockReturnValue(new Promise(() => undefined));
  Object.assign(state, { draft: workflowDraft([]), status: "idle", validation: null, progress: null, report: null, outputDir: null, error: null, startedAt: null });
});
afterEach(() => hooks.mount());

describe("Cube Prep step tabs", () => {
  it("changes only the pixel-area rule of Align when Units changes under the scientific profile (PREPUI-01)", () => {
    const align: CubePrepStep = { step: "align", reference: "F444W", method: "bicubic", border_crop: 1, area_rule: "scale" };
    const match: CubePrepStep = { step: "match", reference: "F444W", mode: "lightstack" };
    state.draft = workflowDraft([{ step: "units", to: "Jy/pixel" }, align, { step: "cube", reference: "F444W", output: "cube.fits", layout: "mef" }, match]);
    const tree = openTab("Units");
    (editors(tree)[0].onChange as unknown as (step: CubePrepStep) => void)({ step: "units", to: "MJy/sr" });
    expect(lastDraft().recipe.steps).toEqual([{ step: "units", to: "MJy/sr" }, { ...align, area_rule: "keep" }, { step: "cube", reference: "F444W", output: "cube.fits", layout: "mef" }, match]);
  });

  it("shows every Edit step with its own editor and removes only the one asked for (PREPUI-02)", () => {
    const cube: CubePrepStep = { step: "cube", reference: "F444W", output: "cube.fits" };
    const drop: CubePrepStep = { step: "edit", action: "drop_invalid", threshold: 0, frac_valid: 0.01, use_dq: true };
    const header: CubePrepStep = { step: "edit", action: "header", redshift: 2, ra_center: null, dec_center: null, use_brightest_pixel: false };
    state.draft = workflowDraft([cube, drop, header]);
    const tree = openTab("Edit");
    expect(editors(tree).map((props) => props.step)).toEqual([drop, header]);
    expect(find(tree, (p, type) => type === "label" && text(p.children).startsWith("Enable Edit"))).toBeUndefined();
    find(tree, (p) => p["aria-label"] === "Remove Edit 2")!.onClick!();
    expect(lastDraft().recipe.steps).toEqual([cube, drop]);
    (editors(render())[0].onChange as unknown as (step: CubePrepStep) => void)({ ...drop, frac_valid: 0.2 });
    expect(lastDraft().recipe.steps).toEqual([cube, { ...drop, frac_valid: 0.2 }]);
  });

  it("adds an Edit step beside the existing one and writes a redshift from the Cube tab (PREPUI-02, proposal 12)", () => {
    const cube: CubePrepStep = { step: "cube", reference: "F444W", output: "cube.fits" };
    const drop: CubePrepStep = { step: "edit", action: "drop_invalid", threshold: 0, frac_valid: 0.01, use_dq: true };
    state.draft = workflowDraft([cube, drop]);
    button(openTab("Edit"), "Add edit")!.onClick!();
    expect(lastDraft().recipe.steps.map((step) => step.action ?? step.step)).toEqual(["cube", "drop_invalid", "drop_invalid"]);
    state.draft = workflowDraft([cube, drop]);
    hooks.mount();
    field(openTab("Cube"), "Redshift (written to the cube header)")!.onChange!({ target: { value: "2.5" } });
    expect(lastDraft().recipe.steps).toEqual([cube, drop, { step: "edit", action: "header", redshift: 2.5, ra_center: null, dec_center: null, use_brightest_pixel: false }]);
    field(render(), "Redshift (written to the cube header)")!.onChange!({ target: { value: "3" } });
    expect(lastDraft().recipe.steps.at(-1)).toMatchObject({ action: "header", redshift: 3 });
    expect(lastDraft().recipe.steps).toHaveLength(3);
  });

  it("keeps step references when a filter is cleared and retyped (PREPUI-17)", () => {
    state.draft = workflowDraft([{ step: "align", reference: "F444W" }, { step: "cube", output: "cube.fits" }]);
    field(render(), "Filter 2")!.onChange!({ target: { value: "" } });
    field(render(), "Filter 2")!.onChange!({ target: { value: "F410M" } });
    expect(lastDraft().recipe.steps).toEqual([{ step: "align", reference: "F410M" }, { step: "cube", output: "cube.fits" }]);
  });

  it("numbers the enabled steps in run order, offers the others to add and marks the tabs that need attention (proposal 5)", () => {
    state.draft = workflowDraft([{ step: "units", to: "Jy/pixel" }, { step: "align", reference: "" }, { step: "cube", output: "cube.fits" }]);
    const tree = render();
    const nav = find(tree, (p) => p["aria-label"] === "Preparation steps")!;
    expect(text(nav.children)).toContain("1. Units2. Align, needs attention3. Cube");
    expect(text(nav.children)).toContain("Add a step:");
    find(tree, (p) => p["aria-label"] === "Add the Crop step")!.onClick!();
    expect(lastDraft().recipe.steps[0]).toMatchObject({ step: "crop", mode: "common" });
    expect(tabButton(render(), "Crop")?.["aria-pressed"]).toBe(true);
    find(render(), (p, type) => type === "button" && text(p.children) === "Choose a reference filter for Align.")!.onClick!();
    expect(tabButton(render(), "Align")?.["aria-pressed"]).toBe(true);
  });
});

describe("Cube Prep goal, reference band and run", () => {
  it("asks what the cube is for and builds the SAGUI chain when SAGUI is chosen (proposal 1)", () => {
    state.draft = workflowDraft([{ step: "align", reference: "F444W", method: "bilinear", border_crop: 1, area_rule: "lightstack" }, { step: "cube", reference: "F444W", output: "cube.fits", layout: "lightstack" }], "lightstack");
    const goal = field(render(), "Cube for")!;
    expect(goal.value).toBe("lightstack");
    goal.onChange!({ target: { value: "scientific" } });
    expect(lastDraft().profile).toBe("scientific");
    expect(lastDraft().recipe.steps.map((step) => step.step)).toEqual(["align", "cube", "psfs", "kernels", "match"]);
    expect(lastDraft().recipe.steps[3]).toMatchObject({ reference: "F444W", auto: true });
  });

  it("sets one reference band for every step from the Inputs tab, defaulting to the broadest PSF (proposal 2)", async () => {
    mocks.filters.mockResolvedValue(REGISTRY);
    state.draft = { ...createCubePrepDraft(), baseDir: "/data", outputParent: "/runs" };
    render();
    await tick();
    find(render(), (p, type) => type === "button" && text(p.children) === "Add FITS images");
    state.draft = { ...state.draft, recipe: { ...state.draft.recipe, inputs: [{ ...INPUTS[0], filter: "" }, INPUTS[1]] } };
    field(render(), "Filter 1")!.onChange!({ target: { value: "F200W" } });
    expect(lastDraft().recipe.steps.filter((step) => "reference" in step).map((step) => step.reference)).toEqual(["F444W", "F444W", "F444W", "F444W", "F444W"]);
    const reference = field(render(), "Reference band (all steps)")!;
    expect(reference.value).toBe("F444W");
    reference.onChange!({ target: { value: "F200W" } });
    expect(lastDraft().recipe.steps.filter((step) => "reference" in step).map((step) => step.reference)).toEqual(["F200W", "F200W", "F200W", "F200W", "F200W"]);
    expect(text(openTab("Kernels"))).toContain("Reference band F200W, set on Inputs. A different reference below applies to this step only.");
  });

  it("offers the registry filters of the dataset with their pivot and a way to type another (proposal 7)", async () => {
    mocks.filters.mockResolvedValue(REGISTRY);
    state.draft = workflowDraft([{ step: "align", reference: "F444W" }, { step: "cube", output: "cube.fits" }]);
    render();
    await tick();
    const filter = field(render(), "Filter 2")!;
    expect(filter.value).toBe("F444W");
    expect(texts(filter.children)).toEqual(["F200W (1.989 µm)", "F410M (4.082 µm)", "F444W (4.404 µm)", "Other filter…"]);
    filter.onChange!({ target: { value: "F410M" } });
    expect(lastDraft().recipe.inputs[1].filter).toBe("F410M");
    field(render(), "Filter 2")!.onChange!({ target: { value: "__other" } });
    const typed = field(render(), "Filter 2")!;
    expect(typed.value).toBe("F410M");
    expect(typed.children).toBeUndefined();
  });

  it("captions the Run button with the first problem, links it to the list and keeps Validate as a secondary action (proposal 6)", () => {
    state.draft = workflowDraft([{ step: "align", reference: "" }, { step: "cube", output: "cube.fits" }]);
    const tree = render();
    const run = find(tree, (p) => "runningLabel" in p)!;
    expect(run.label).toBe("Choose a reference filter for Align.");
    expect(run.disabled).toBe(true);
    const list = find(tree, (p, type) => type === "ul" && typeof p.id === "string")!;
    expect(run.describedBy).toBe(list.id);
    state.draft = workflowDraft([{ step: "align", reference: "F444W" }, { step: "cube", output: "cube.fits" }]);
    const ready = find(render(), (p) => "runningLabel" in p)!;
    expect(ready).toMatchObject({ label: "Validate & run", disabled: false, describedBy: undefined });
    button(render(), "Validate")!.onClick!();
    expect(mocks.validate).toHaveBeenCalledOnce();
  });

  it("shows the input folder only when it is needed and names where runs are saved (proposal 11, PREPUI-16)", () => {
    state.draft = { ...workflowDraft([{ step: "align", reference: "F444W" }, { step: "cube", output: "cube.fits" }]), baseDir: "" };
    const missing = render();
    expect(field(missing, "Input folder (relative recipe paths)")).toBeDefined();
    expect(find(missing, (p) => "runningLabel" in p)).toMatchObject({ label: "Choose the input folder for relative file paths.", disabled: true });
    state.draft = workflowDraft([{ step: "align", reference: "F444W" }, { step: "cube", output: "cube.fits" }]);
    const absolute = render();
    expect(field(absolute, "Input folder (relative recipe paths)")).toBeUndefined();
    expect(field(absolute, "Save runs in")?.value).toBe("/runs");
    expect(text(absolute)).toContain("Each run creates a folder cubeprep-field-<date and time> here");
    state.draft = { ...state.draft, recipe: { ...state.draft.recipe, inputs: [{ path: "f200w.fits", dataset: "JWST", filter: "F200W" }] } };
    expect(field(render(), "Input folder (relative recipe paths)")?.value).toBe("/data");
  });

  it("lists the steps of a running recipe with done, current and pending marks (proposal 24)", () => {
    state.draft = workflowDraft([{ step: "units", to: "Jy/pixel" }, { step: "align", reference: "F444W" }, { step: "cube", output: "cube.fits" }]);
    Object.assign(state, { status: "running", startedAt: 1, progress: { stage: "02_align", current: 1, total: 3, percent: 40 } });
    const list = find(render(), (p) => "data-run-steps" in p)!;
    expect(findAll(list.children, (p) => "data-step-state" in p).map((p) => [p["data-step-state"], text(p.children)])).toEqual([["done", "✓ 1. Units, done"], ["current", "▸ 2. Align, running"], ["pending", "○ 3. Cube, pending"]]);
  });
});

describe("Cube Prep result", () => {
  const steps = [
    { index: 1, step: "cube", directory: "01_cube", parameters: {}, warnings: [], elapsed_ms: 1, report: { cols: 43, rows: 41, layout: "mef", hdus: ["SCI", "ERR", "DQ", "FILTERS"], planes: [{ filter: "F200W", pivot_um: 1.9886 }, { filter: "F444W", pivot_um: 4.4043 }] } },
    { index: 2, step: "kernels", directory: "02_kernels", parameters: { auto: false }, warnings: [], elapsed_ms: 1, report: { reference: "F444W", pixel_scale_arcsec: 0.04, kernels: [{ filter: "F200W", diagnostics: { source_fwhm_px: 2.5, target_fwhm_px: 3.471, matched_fwhm_px: 4.21 } }] } },
    { index: 3, step: "match", directory: "03_match", parameters: {}, warnings: [], elapsed_ms: 1, report: { reference: "F444W", planes: [{ filter: "F200W", status: "matched" }, { filter: "F444W", status: "reference" }] } },
  ];

  it("summarises the cube first, marks the bands that miss the target FWHM and keeps step details under Diagnostics (proposals 3, 4)", () => {
    Object.assign(state, { status: "complete", report: { ...report, steps } });
    const tree = render();
    const summary = find(tree, (p) => "data-cube-summary" in p)!;
    expect(texts(summary.children)).toEqual(["Final cube", "2 bands: F200W (1.989 µm), F444W (4.404 µm)", "43 × 41 pixels · MEF · SCI, ERR, DQ, FILTERS", "SAGUI PSF target not set: F200W +21.3 % from the target FWHM (tolerance 2 %). Turn on the automatic choice in Kernels and run again."]);
    const table = find(tree, (p) => "data-psf-fwhm" in p)!;
    expect(texts(table.children)).toContain("vs target");
    expect(find(table.children, (p) => p["data-outside-tolerance"] === true)).toBeDefined();
    expect(texts(table.children)).toContain("+21.3 %");
    expect(text(find(tree, (p) => "data-diagnostics" in p)!.children)).toContain("1. cube");
  });

  it("opens SAGUI from the result as the next step (proposal 23)", async () => {
    Object.assign(state, { status: "complete", report: { ...report, steps } });
    mocks.prepare.mockResolvedValueOnce(true);
    button(render(), "Send to SAGUI")!.onClick!();
    await vi.waitFor(() => expect(mocks.dispatch).toHaveBeenCalledWith({ type: "open", tool: "sagui" }));
  });
});

describe("Cube Prep inputs", () => {
  it("shows what to do first on an empty recipe (proposal 10)", () => {
    state.draft = { ...createCubePrepDraft(), baseDir: "", outputParent: "" };
    expect(texts(find(render(), (p) => "data-empty-inputs" in p)!.children)).toEqual(["Add one image per filter with Add FITS images, or Use selected image for the file selected in Files.", "Check the dataset and filter of each image. The reference band defaults to the broadest PSF.", "Choose a PSF file per filter in PSFs, then Validate & run."]);
  });

  it("offers the band with the broadest measured PSF as reference once every input is measured (PREPUI-19)", async () => {
    state.draft = workflowDraft([{ step: "align", reference: "F444W" }, { step: "cube", output: "cube.fits" }]);
    mocks.psf.mockResolvedValueOnce({ fwhm_px: 6, fwhm_arcsec: 0.19, fwhm_px_scatter: null, n_stars: 9, warnings: [] } as unknown as ImagePsfFwhm).mockResolvedValueOnce({ fwhm_px: 3, fwhm_arcsec: 0.14, fwhm_px_scatter: null, n_stars: 9, warnings: [] } as unknown as ImagePsfFwhm);
    button(render(), "Measure PSFs")!.onClick!();
    await vi.waitFor(() => expect(mocks.psf).toHaveBeenCalledTimes(2));
    await tick();
    const tree = render();
    expect(text(tree)).toContain("The measured PSF is broadest in F200W.");
    button(tree, "Use F200W as reference")!.onClick!();
    expect(lastDraft().recipe.steps.map((step) => step.reference)).toEqual(["F200W", undefined]);
  });
});
