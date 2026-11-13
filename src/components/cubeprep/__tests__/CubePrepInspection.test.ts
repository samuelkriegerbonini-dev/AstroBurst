import { isValidElement, type ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { FitsExtension } from "../../../services/header";
import type { CubePrepReport, CubePrepStep, CubePrepStepReport } from "../../../shared/types/cubeprep";
import type { CompanionSuggestions } from "../../../shared/types/layers";
import type { ImagePsfFwhm, ResolutionCheck, ResolutionRequest } from "../../../shared/types/resolution";
import type { CubePrepState } from "../cubePrepStore";
import { createCubePrepDraft, type CubePrepDraft } from "../cubePrepModel";

type OpenedFile = { id: string; path: string };
const mocks = vi.hoisted(() => ({ prepare: vi.fn(), dispatch: vi.fn(), header: vi.fn(), extensions: vi.fn(), filters: vi.fn(), edit: vi.fn(), layers: vi.fn(), wcs: vi.fn(), psf: vi.fn(), check: vi.fn(), open: vi.fn(), files: [] as OpenedFile[], selected: null as OpenedFile | null }));
const report: CubePrepReport = { recipe_version: 1, name: "field", output_dir: "/out", cube: "final.fits", steps: [], files: [], warnings: [], elapsed_ms: 1 };
function inspectedDraft(): CubePrepDraft {
  const draft = createCubePrepDraft();
  draft.baseDir = "/data";
  draft.recipe.inputs = [{ path: "f200w.fits.gz", dataset: "JWST", filter: "F200W" }, { path: "f444w.fits.gz", dataset: "JWST", filter: "F444W" }];
  return draft;
}
const state: CubePrepState = { draft: inspectedDraft(), status: "complete", validation: null, progress: null, report, outputDir: "/out", error: null, startedAt: null };
vi.mock("react", async (original) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await original()));
vi.mock("../cubePrepStore", () => ({ useCubePrepState: () => state, cubePrepStore: { get: () => state, edit: mocks.edit, isJobActive: () => false } }));
vi.mock("../../sagui/saguiStore", () => ({ saguiStore: { prepareCubePrep: mocks.prepare } }));
vi.mock("../../../hooks/useDockLayout", () => ({ dockStore: { dispatch: mocks.dispatch } }));
vi.mock("../../../hooks/useFileStore", () => ({ fileStore: { getFiles: () => mocks.files, getSelectedFile: () => mocks.selected } }));
vi.mock("../../../hooks/useFileIngest", () => ({ showWhenLoaded: vi.fn() }));
vi.mock("../../../services/cubeprep", () => ({ cubePrepFilters: mocks.filters }));
vi.mock("../../../services/header", () => ({ getHeader: mocks.header, getFitsExtensions: mocks.extensions }));
vi.mock("../../../services/companions", () => ({ suggestCompanions: mocks.layers }));
vi.mock("../../../services/astrometry", () => ({ getWcsInfo: mocks.wcs }));
vi.mock("../../../services/resolution", () => ({ imagePsfFwhm: mocks.psf, resolutionCheck: mocks.check }));
vi.mock("../../../hooks/useResolutionSettings", async () => {
  const { DEFAULT_RESOLUTION_SETTINGS } = await import("../../../utils/resolutionText");
  return { useResolutionSettings: () => DEFAULT_RESOLUTION_SETTINGS };
});
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: mocks.open, save: vi.fn() }));
import { hooks } from "../../../test/hookRuntime";
import { DEFAULT_RESOLUTION_SETTINGS } from "../../../utils/resolutionText";
import CubePrepPanel from "../CubePrepPanel";
import { redshiftRequest } from "../cubePrepInspect";

type Props = Record<string, unknown> & { children?: ReactNode; onClick?: () => void };
function find(node: ReactNode, match: (props: Props) => boolean): Props | undefined {
  if (Array.isArray(node)) { for (const child of node) { const hit = find(child, match); if (hit) return hit; } return undefined; }
  if (!isValidElement<Props>(node)) return undefined;
  if (match(node.props)) return node.props;
  return find(node.props.children, match);
}
function texts(node: ReactNode): string[] {
  if (typeof node === "string") return [node];
  if (Array.isArray(node)) return node.flatMap(texts);
  if (!isValidElement<Props>(node)) return [];
  return texts(node.props.children);
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
const render = () => hooks.render(CubePrepPanel, {}) as ReactNode;
const button = (tree: ReactNode, label: string | RegExp) => find(tree, (p) => typeof p.children === "string" && (typeof label === "string" ? p.children === label : label.test(p.children)));
const psfButton = (tree: ReactNode) => button(tree, /^(Measure PSFs|Measuring PSF \d+ of \d+…)$/);
const editor = (tree: ReactNode) => find(tree, (p) => "step" in p && "recipe" in p && "browse" in p);
const fwhmTable = (tree: ReactNode) => find(tree, (p) => "data-psf-fwhm" in p);
const count = (lines: string[], text: string) => lines.filter((line) => line === text).length;
const tick = () => new Promise((done) => setTimeout(done, 0));

function hdu(path: string, index: number, extname: string | null, more: Partial<FitsExtension> = {}): FitsExtension {
  return { index, extname, naxis: 2, naxis1: 10, naxis2: 10, naxis3: 0, bitpix: -32, has_data: index > 0, extver: null, ref: `${path}#hdu=${index}`, kind: "hdu", is_err: false, is_dq: false, ...more };
}
const mef = (path: string) => [hdu(path, 0, null), hdu(path, 1, "SCI"), hdu(path, 2, "ERR", { is_err: true, role: "sigma" }), hdu(path, 3, "DQ", { is_dq: true, role: "mask" })];
const layersOf = (path: string): CompanionSuggestions => ({
  role: { role: "science", basis: "t", assumed: false },
  err: { source: `${path}#hdu=2`, role: "sigma", basis: "EXTNAME ERR" },
  dq: { source: `${path}#hdu=3`, basis: "EXTNAME DQ" },
  applied: true,
  candidates: { err: [], dq: [] },
  notes: [],
});
const LAYERS = "MJy/sr · error: ERR, HDU 2 (sigma) · mask: DQ, HDU 3";
const FALLBACK = "MJy/sr · ERR present · DQ present";
const SCALE = 'Pixel scale 0.03100"/px';
const NO_WCS = "No celestial WCS: pixel scale unknown";
const PSF = { fwhm_px: 3.506, fwhm_arcsec: 0.9184, fwhm_px_scatter: 0.13, n_stars: 108, warnings: ["w-psf"] } as unknown as ImagePsfFwhm;
const PSF_LINE = "PSF FWHM 3.51 px = 0.918 arcsec ± 0.13 px, 108 stars";
const OTHER_PSF = { fwhm_px: 4.2, fwhm_arcsec: 1.1, fwhm_px_scatter: null, n_stars: 1, warnings: [] } as unknown as ImagePsfFwhm;
const OTHER_PSF_LINE = "PSF FWHM 4.20 px = 1.1 arcsec, 1 star";
const STEPS = [
  { step: "psfs", report: { prepared: { psfs: [{ filter: "F200W", pixel_scale_arcsec: 0.03, after: { fwhm_px: 2.2, fwhm_arcsec: 0.066 } }, { filter: "F444W", pixel_scale_arcsec: 0.03, after: { fwhm_px: 4.8, fwhm_arcsec: 0.144 } }] } } },
  { step: "kernels", report: { reference: "f444w", pixel_scale_arcsec: 0.03, kernels: [{ filter: "F200W", diagnostics: { source_fwhm_px: 2.2, target_fwhm_px: 4.8, matched_fwhm_px: 4.83 } }] } },
];
const STEP_REPORTS: CubePrepStepReport[] = STEPS.map((entry, index) => ({ index: index + 6, step: entry.step, directory: `0${index + 6}_${entry.step}`, parameters: {}, report: entry.report, warnings: [], elapsed_ms: 10 }));
const HEADER: CubePrepStep = { step: "edit", action: "header", redshift: 2, ra_center: null, dec_center: null, use_brightest_pixel: false };
const COSMOLOGY = "flat LambdaCDM, H0 67.66, Omega_m 0.30966, plus Planck 2018 neutrinos and radiation";
const CHECK: ResolutionCheck = {
  verdict: null, size_basis: null, psf_fwhm_arcsec: null, psf_reference: null, pixels_per_fwhm: null, nyquist_sampled: null, psf_area_px: null,
  intrinsic_size_arcsec: null, snr_floor_fraction: null, psf_floor_fraction: 0.24677925358506123, floor_fraction: null, size_floor_arcsec: null, resolution_elements: null,
  kpc_per_arcsec: 8.578275309405592, kpc_per_pixel: null, angular_diameter_distance_mpc: 1769.396294628795, psf_fwhm_kpc: null, intrinsic_size_kpc: null, size_floor_kpc: null,
  scale_of_interest_in_psf: null, snr: null, redshift: 2, psf_fwhm_rel_err: 0.03, resolved_elements: 3, min_pixels_per_fwhm: 2, scale_of_interest_kpc: 1, h0_km_s_mpc: 67.66, omega_m: 0.30966,
  omega_nu_massive: 0.0014147613306646316, omega_rad: 7.893312479128647e-5, cosmology: COSMOLOGY, notes: [], warnings: [],
};

function openEdit(): ReactNode {
  find(render(), (p) => Array.isArray(p.children) && p.children[0] === "Edit")!.onClick!();
  return render();
}

beforeEach(() => {
  hooks.mount();
  vi.clearAllMocks();
  for (const mock of [mocks.header, mocks.extensions, mocks.layers, mocks.wcs, mocks.psf, mocks.check, mocks.open]) mock.mockReset();
  mocks.filters.mockReset().mockResolvedValue({ datasets: [], filters: [] });
  mocks.files = [];
  mocks.selected = null;
  state.draft = inspectedDraft();
  state.report = report;
});
afterEach(() => hooks.mount());

describe("Cube Prep input inspection", () => {
  it("Inspect inputs names the layers in use and the pixel scale", async () => {
    mocks.header.mockResolvedValue({ BUNIT: "MJy/sr" });
    mocks.extensions.mockImplementation(async (path: string) => ({ extensions: mef(path) }));
    mocks.layers.mockImplementation(async (path: string) => layersOf(path));
    mocks.wcs.mockResolvedValue({ pixel_scale_arcsec: 0.031 });
    button(render(), "Inspect inputs")!.onClick!();
    await tick();
    await tick();
    const lines = texts(render());
    expect(count(lines, LAYERS)).toBe(2);
    expect(count(lines, SCALE)).toBe(2);
    for (const mock of [mocks.layers, mocks.wcs]) expect(mock.mock.calls.map(([path]) => path)).toEqual(["/data/f200w.fits.gz", "/data/f444w.fits.gz"]);
  });

  it("a file without a celestial WCS or without suggestions keeps its other lines", async () => {
    mocks.header.mockResolvedValue({ BUNIT: "MJy/sr" });
    mocks.extensions.mockResolvedValue({ extensions: [{ is_err: true, is_dq: true }] });
    mocks.layers.mockRejectedValue(new Error("no suggestions"));
    mocks.wcs.mockRejectedValue(new Error("no celestial WCS"));
    button(render(), "Inspect inputs")!.onClick!();
    await tick();
    await tick();
    const lines = texts(render());
    expect(count(lines, FALLBACK)).toBe(2);
    expect(count(lines, NO_WCS)).toBe(2);
    mocks.header.mockReset().mockResolvedValueOnce({ BUNIT: "MJy/sr" }).mockRejectedValueOnce(new Error("header unreadable"));
    button(render(), "Inspect inputs")!.onClick!();
    await tick();
    await tick();
    const after = texts(render());
    expect(count(after, FALLBACK)).toBe(1);
    expect(count(after, NO_WCS)).toBe(1);
    expect(count(after, "header unreadable")).toBe(1);
  });
});

describe("Cube Prep PSF of each input", () => {
  it("Measure PSFs measures one input at a time", async () => {
    const first = deferred<ImagePsfFwhm>();
    const second = deferred<ImagePsfFwhm>();
    mocks.psf.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    const before = render();
    expect(psfButton(before)).toMatchObject({ children: "Measure PSFs", title: "Measures the stars of each input, one image at a time; large images take a while" });
    psfButton(before)!.onClick!();
    expect(mocks.psf.mock.calls).toEqual([["/data/f200w.fits.gz"]]);
    expect(psfButton(render())).toMatchObject({ children: "Measuring PSF 1 of 2…", disabled: true });
    first.resolve(PSF);
    await tick();
    expect(mocks.psf.mock.calls).toEqual([["/data/f200w.fits.gz"], ["/data/f444w.fits.gz"]]);
    const during = render();
    expect(texts(during)).toContain(PSF_LINE);
    expect(find(during, (p) => JSON.stringify(p.warnings) === '["w-psf"]')).toBeDefined();
    expect(psfButton(during)).toMatchObject({ children: "Measuring PSF 2 of 2…", disabled: true });
  });

  it("a PSF that cannot be measured does not stop the others and is said", async () => {
    const first = deferred<ImagePsfFwhm>();
    const second = deferred<ImagePsfFwhm>();
    mocks.psf.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    psfButton(render())!.onClick!();
    first.reject(new Error("No star was detected in this image"));
    await tick();
    second.resolve(OTHER_PSF);
    await tick();
    const after = render();
    expect(texts(after)).toContain("PSF not measured: No star was detected in this image");
    expect(texts(after)).toContain(OTHER_PSF_LINE);
    expect(psfButton(after)).toMatchObject({ children: "Measure PSFs", disabled: false });
  });

  it("the measurement stops when the inputs change or the panel closes", async () => {
    const first = deferred<ImagePsfFwhm>();
    mocks.psf.mockReturnValueOnce(first.promise).mockReturnValueOnce(deferred<ImagePsfFwhm>().promise);
    psfButton(render())!.onClick!();
    state.draft.recipe.inputs = [{ path: "f090w.fits.gz", dataset: "JWST", filter: "F090W" }];
    first.resolve(PSF);
    await tick();
    const after = render();
    expect(mocks.psf).toHaveBeenCalledOnce();
    expect(texts(after)).not.toContain(PSF_LINE);
    expect(psfButton(after)).toMatchObject({ children: "Measure PSFs", disabled: false });

    hooks.mount();
    state.draft = inspectedDraft();
    mocks.psf.mockReset();
    const closed = deferred<ImagePsfFwhm>();
    mocks.psf.mockReturnValueOnce(closed.promise).mockReturnValueOnce(deferred<ImagePsfFwhm>().promise);
    psfButton(render())!.onClick!();
    hooks.mount();
    closed.resolve(PSF);
    await tick();
    expect(mocks.psf).toHaveBeenCalledOnce();
  });
});

describe("Cube Prep report and Edit step", () => {
  it("the report shows source, target and matched FWHM per filter", () => {
    state.report = { ...report, steps: STEP_REPORTS };
    const table = fwhmTable(render());
    expect(table).toBeDefined();
    const cells = texts(table!.children);
    for (const text of ["PSF FWHM by filter: source, target and after matching", "Filter", "Source", "Target", "Matched", "F200W", '2.20 px (0.066")', '4.80 px (0.144")', '4.83 px (0.145")', "F444W", "reference", "--"]) expect(cells).toContain(text);
    state.report = { ...report, steps: [] };
    expect(fwhmTable(render())).toBeUndefined();
  });

  it("the Edit step gets kpc per arcsec for its redshift", async () => {
    state.draft.recipe.steps = [HEADER];
    const answer = deferred<ResolutionCheck>();
    mocks.check.mockReturnValueOnce(answer.promise);
    openEdit();
    expect(mocks.check).toHaveBeenCalledOnce();
    expect(mocks.check).toHaveBeenCalledWith(redshiftRequest(2, DEFAULT_RESOLUTION_SETTINGS, null));
    answer.resolve(CHECK);
    await tick();
    expect(editor(render())?.redshiftNote).toEqual({ scale: "8.58 kpc/arcsec at z = 2", psf: null, cosmology: COSMOLOGY });
    mocks.check.mockReturnValueOnce(deferred<ResolutionCheck>().promise);
    state.report = { ...report, steps: STEP_REPORTS };
    const waiting = render();
    expect(mocks.check).toHaveBeenCalledTimes(2);
    const asked = mocks.check.mock.calls[1][0] as ResolutionRequest;
    expect(asked.psf_fwhm_arcsec).toBeCloseTo(0.144, 12);
    expect(asked.psf_reference).toBe("target PSF of the last Cube Prep run");
    expect(editor(waiting)?.redshiftNote).toBeNull();
    expect(editor(render())?.redshiftNote).toBeNull();

    hooks.mount();
    mocks.check.mockClear();
    state.draft.recipe.steps = [{ ...HEADER, redshift: null }];
    const tree = openEdit();
    expect(mocks.check).not.toHaveBeenCalled();
    expect(editor(tree)?.redshiftNote).toBeNull();
  });

  it("bzip2 FITS files are accepted like gzip ones", () => {
    mocks.selected = { id: "5", path: "/data/a.fits.bz2" };
    mocks.files = [mocks.selected];
    button(render(), "Use selected image")!.onClick!();
    expect(mocks.edit).toHaveBeenCalledOnce();
    const draft = mocks.edit.mock.calls[0][0] as CubePrepDraft;
    expect(draft.recipe.inputs.at(-1)?.path).toBe("/data/a.fits.bz2");
    mocks.open.mockResolvedValue(null);
    button(render(), "Add FITS images")!.onClick!();
    expect(mocks.open).toHaveBeenCalledWith({ multiple: true, filters: [{ name: "FITS images", extensions: ["fits", "fit", "fts", "gz", "bz2", "fz"] }] });
  });
});
