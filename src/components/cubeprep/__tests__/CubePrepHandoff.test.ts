import { isValidElement, type ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CubePrepReport } from "../../../shared/types/cubeprep";
import type { CubePrepState } from "../cubePrepStore";
import { createCubePrepDraft, type CubePrepDraft } from "../cubePrepModel";

type OpenedFile = { id: string; path: string };
const mocks = vi.hoisted(() => ({ prepare: vi.fn(), dispatch: vi.fn(), header: vi.fn(), extensions: vi.fn(), filters: vi.fn(), edit: vi.fn(), files: [] as OpenedFile[], selected: null as OpenedFile | null }));
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
import { hooks } from "../../../test/hookRuntime";
import CubePrepPanel from "../CubePrepPanel";

type Props = Record<string, unknown> & { children?: ReactNode; onClick?: () => void; onChange?: (event: { target: { value: string } }) => void };
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
  const promise = new Promise<T>((yes) => { resolve = yes; });
  return { promise, resolve };
}
const RESULT = "MJy/sr · ERR present · DQ present";
const render = () => hooks.render(CubePrepPanel, {}) as ReactNode;
const button = (tree: ReactNode, label: string | RegExp) => find(tree, (p) => typeof p.children === "string" && (typeof label === "string" ? p.children === label : label.test(p.children)));
const sendButton = (tree: ReactNode) => button(tree, /^(Send to SAGUI|Checking cube…)$/);
const inspectButton = (tree: ReactNode) => button(tree, /^Inspect/);
const field = (tree: ReactNode, label: string) => {
  const holder = find(tree, (p) => p.label === label);
  return isValidElement<Props>(holder?.children) ? holder.children.props : undefined;
};
const alert = (tree: ReactNode) => find(tree, (p) => "message" in p && typeof p.message === "string")?.message;
const tick = () => new Promise((done) => setTimeout(done, 0));

function startInspection() {
  const header = deferred<Record<string, string>>();
  const extensions = deferred<{ extensions: { is_err: boolean; is_dq: boolean }[] }>();
  mocks.header.mockImplementation(() => header.promise);
  mocks.extensions.mockImplementation(() => extensions.promise);
  const first = render();
  expect(inspectButton(first)?.children).toBe("Inspect inputs");
  inspectButton(first)!.onClick!();
  const during = render();
  expect(inspectButton(during)).toMatchObject({ children: "Inspecting…", disabled: true });
  return {
    during,
    finish: async () => { header.resolve({ BUNIT: "MJy/sr" }); extensions.resolve({ extensions: [{ is_err: true, is_dq: true }] }); await tick(); await tick(); },
  };
}

beforeEach(() => {
  hooks.mount();
  vi.clearAllMocks();
  mocks.header.mockReset();
  mocks.extensions.mockReset();
  mocks.filters.mockReset().mockResolvedValue({ datasets: [], filters: [] });
  mocks.files = [];
  mocks.selected = null;
  state.draft = inspectedDraft();
  state.report = report;
});
afterEach(() => hooks.mount());

describe("Cube Prep to SAGUI desktop action", () => {
  it("opens the existing dock tool only after successful input preparation", async () => {
    let resolve!: (accepted: boolean) => void;
    mocks.prepare.mockReturnValueOnce(new Promise<boolean>((yes) => { resolve = yes; }));
    const click = sendButton(render())?.onClick;
    expect(click).toBeDefined();
    click!();
    expect(mocks.prepare).toHaveBeenCalledWith(report, expect.any(Function));
    expect(mocks.dispatch).not.toHaveBeenCalled();
    resolve(true);
    await vi.waitFor(() => expect(mocks.dispatch).toHaveBeenCalledWith({ type: "open", tool: "sagui" }));
  });

  it("makes changed reports and an unmounted sender obsolete without opening the dock", async () => {
    mocks.prepare.mockResolvedValueOnce(false);
    sendButton(render())!.onClick!();
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

  it.each([
    ["is refused", () => mocks.prepare.mockRejectedValueOnce(new Error("SAGUI requires a multiband cube"))],
    ["is dropped", () => mocks.prepare.mockResolvedValueOnce(false)],
    ["is accepted", () => mocks.prepare.mockResolvedValueOnce(true)],
  ])("re-enables Inspect when a hand-off that %s ends during the inspection (SCIENCE-01)", async (_outcome, settle) => {
    settle();
    const inspection = startInspection();
    sendButton(inspection.during)!.onClick!();
    await tick();
    expect(sendButton(render())?.children).toBe("Send to SAGUI");
    await inspection.finish();
    const after = render();
    expect(inspectButton(after)).toMatchObject({ children: "Inspect inputs", disabled: false });
    expect(texts(after).filter((text) => text === RESULT)).toHaveLength(2);
  });

  it("keeps a pending hand-off current when Inspect starts (SCIENCE-01)", async () => {
    let resolve!: (accepted: boolean) => void;
    mocks.prepare.mockReturnValueOnce(new Promise<boolean>((yes) => { resolve = yes; }));
    sendButton(render())!.onClick!();
    const current = mocks.prepare.mock.calls[0][1] as () => boolean;
    const inspection = startInspection();
    expect(current()).toBe(true);
    resolve(true);
    await inspection.finish();
    expect(mocks.dispatch).toHaveBeenCalledWith({ type: "open", tool: "sagui" });
    hooks.mount();
    expect(current()).toBe(false);
  });

  it("says why a dropped hand-off did not open SAGUI (SCIENCE-14)", async () => {
    mocks.prepare.mockResolvedValueOnce(false);
    sendButton(render())!.onClick!();
    await tick();
    expect(alert(render())).toBe("SAGUI changed while the cube was being checked; send again.");
    let resolve!: (accepted: boolean) => void;
    mocks.prepare.mockReturnValueOnce(new Promise<boolean>((yes) => { resolve = yes; }));
    sendButton(render())!.onClick!();
    state.report = { ...report };
    resolve(false);
    await tick();
    expect(alert(render())).toBe("Recipe changed; send again.");
    mocks.prepare.mockReturnValueOnce(new Promise<boolean>((yes) => { resolve = yes; }));
    sendButton(render())!.onClick!();
    state.report = null;
    resolve(false);
    await tick();
    expect(alert(render())).toBe("Recipe changed; run it again before sending to SAGUI.");
    expect(mocks.dispatch).not.toHaveBeenCalled();
  });
});

describe("Cube Prep inputs", () => {
  it("keeps a failed filter registry load on screen with Retry (SCIENCE-20)", async () => {
    mocks.filters.mockRejectedValueOnce(new Error("registry unavailable"));
    render();
    await tick();
    const failed = render();
    expect(alert(failed)).toBe("Could not load the filter registry: registry unavailable");
    field(failed, "Recipe name")!.onChange!({ target: { value: "renamed" } });
    expect(mocks.edit).toHaveBeenCalled();
    const edited = render();
    expect(alert(edited)).toBe("Could not load the filter registry: registry unavailable");
    mocks.filters.mockResolvedValueOnce({ datasets: ["JWST"], filters: [] });
    button(edited, "Retry")!.onClick!();
    render();
    await tick();
    const loaded = render();
    expect(mocks.filters).toHaveBeenCalledTimes(2);
    expect(button(loaded, "Retry")).toBeUndefined();
    expect(alert(loaded)).toBeUndefined();
    expect(find(loaded, (p) => p.id === "cubeprep-datasets")?.children).toHaveLength(1);
  });

  it("adds only the selected image and says how many opened images were skipped (SCIENCE-21)", () => {
    mocks.files = [{ id: "1", path: "/data/a.fits" }, { id: "2", path: "/data/dark.fits" }, { id: "3", path: "/data/master.fits.gz" }, { id: "4", path: "/data/notes.png" }];
    mocks.selected = mocks.files[2];
    const tree = render();
    expect(button(tree, "Use opened images")).toBeUndefined();
    button(tree, "Use selected image")!.onClick!();
    expect(mocks.edit).toHaveBeenCalledOnce();
    const draft = mocks.edit.mock.calls[0][0] as CubePrepDraft;
    expect(draft.recipe.inputs.map((input) => input.path)).toEqual(["f200w.fits.gz", "f444w.fits.gz", "/data/master.fits.gz"]);
    expect(texts(render())).toContain("Added the selected image. 2 other opened FITS files were skipped.");
    state.draft.recipe.inputs = [...state.draft.recipe.inputs, { path: "/data/master.fits.gz", dataset: "JWST", filter: "F090W" }, { path: "/data/a.fits", dataset: "JWST", filter: "F150W" }];
    button(render(), "Use selected image")!.onClick!();
    expect(mocks.edit).toHaveBeenCalledOnce();
    expect(texts(render())).toContain("The selected image is already an input. 1 other opened FITS file was skipped.");
    mocks.selected = mocks.files[3];
    button(render(), "Use selected image")!.onClick!();
    expect(mocks.edit).toHaveBeenCalledOnce();
    expect(alert(render())).toBe("Select a FITS image in Files to add it.");
  });

  it("renames step references along with the input filter (SCIENCE-16)", () => {
    state.draft.recipe.steps = [{ step: "align", reference: "F444W" }, { step: "cube", output: "cube.fits" }];
    field(render(), "Filter 2")!.onChange!({ target: { value: "F410M" } });
    const draft = mocks.edit.mock.calls[0][0] as CubePrepDraft;
    expect(draft.recipe.inputs[1].filter).toBe("F410M");
    expect(draft.recipe.steps).toEqual([{ step: "align", reference: "F410M" }, { step: "cube", output: "cube.fits" }]);
  });
});
