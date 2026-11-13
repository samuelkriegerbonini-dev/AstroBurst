import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactElement, ReactNode } from "react";
import type { PlaneInfo } from "../../../shared/types/fits.types";
import type { PendingLayer } from "../../../utils/layerLabels";

type FileRow = { id: string; path: string; result: { plane: PlaneInfo } | null };

const mocks = vi.hoisted(() => ({ set: vi.fn(), clear: vi.fn(), sdss: vi.fn(), apply: vi.fn(), open: vi.fn(), pending: vi.fn() }));
const view = vi.hoisted(() => ({ file: null as FileRow | null }));

vi.mock("react", async (original) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await original()));
vi.mock("../../../context/PreviewContext", () => ({ useFileContext: () => ({ file: view.file }) }));
vi.mock("../../../services/companions", () => ({ setCompanion: mocks.set, clearCompanion: mocks.clear, setSdssFrameError: mocks.sdss }));
vi.mock("../../../hooks/applyLayerChange", () => ({ applyLayerChange: mocks.apply }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: mocks.open }));
vi.mock("../../../hooks/useMeasurementLog", () => ({ useMeasurementProvenance: () => ({ file: "a.fits", image: "original" }) }));

import CompanionAssign from "../CompanionAssign";
import ErrorAlert from "../../ui/ErrorAlert";
import WarningList from "../../ui/WarningList";
import { hooks } from "../../../test/hookRuntime";
import { elementsOf, textOf } from "../../../test/elementTree";
import { measurementLog } from "../../../utils/measurementLog";

const PATH = "C:/d/a.fits#hdu=1";
const BASE: PlaneInfo = {
  kind: "hdu",
  index: 1,
  key: null,
  extname: "IMAGE",
  extver: null,
  is_dq: false,
  is_err: false,
  dq_ref: null,
  err_ref: null,
  source_path: "C:/d/a.fits",
  dq_table: null,
};
const IN_USE: PlaneInfo = {
  ...BASE,
  err_ref: "C:/d/a.fits#hdu=3",
  dq_ref: "C:/d/a.fits#hdu=2",
  err_origin: { role: "variance", source: "EXTNAME VARIANCE, HDU 3: square root", assumed: false, n_no_data: 12, origin: "same_file" },
  dq_origin: "EXTNAME MASK, HDU 2",
  dq_origin_kind: "same_file",
};
const RETURNED: PlaneInfo = { ...IN_USE, err_origin: { ...IN_USE.err_origin!, origin: "user" } };
const MISMATCH = "error layer HDU 2 (VARIANCE) is 2x2 and does not match the image 4x4: not used";
const NOTE = "error layer HDU 3: EXTNAME WGT is assumed to be an inverse variance";
const WARNING = "PS1 asinh-scaled pixels were linearised (BOFFSET, BSOFTEN)";
const HAND_SET = "Layers set by hand apply to new measurements. The time series and the log keep what they measured before the change.";
const SDSS_REASON =
  "SDSS frame: the file carries no error plane; it can be rebuilt from the calibration vector (HDU 1) and the sky table (HDU 2) once the gain and the dark variance are typed";
const NOISE2: PendingLayer = { role: "err", source: "C:/d/a.fits#hdu=2", name: "HDU 2 (NOISE2)" };

type Props = Record<string, unknown> & { children?: ReactNode };
type AssignProps = { imagePath: string; plane: PlaneInfo; pending: PendingLayer | null; onPendingChange: (pending: PendingLayer | null) => void };

const component = (CompanionAssign as unknown as { type: (props: AssignProps) => ReactNode }).type;
const render = (plane: PlaneInfo, pending: PendingLayer | null = null, imagePath = PATH) =>
  hooks.render(component, { imagePath, plane, pending, onPendingChange: mocks.pending }) as ReactNode;
const elements = (tree: ReactNode) => elementsOf<Props>(tree);
const buttons = (tree: ReactNode) => elements(tree).filter((e) => e.type === "button");
const named = (tree: ReactNode, label: string) => buttons(tree).filter((b) => textOf(b) === label);
const row = (tree: ReactNode, title: "Error" | "Mask") => elements(tree).find((e) => e.props.role === "group" && e.props["aria-label"] === title)!;
const click = (button: ReactElement<Props> | undefined) => (button!.props.onClick as () => unknown)() as Promise<void> | undefined;
const field = (tree: ReactNode, label: string) => elements(tree).find((e) => e.props["aria-label"] === label)!;
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((yes) => {
    resolve = yes;
  });
  return { promise, resolve };
}

beforeEach(() => {
  hooks.mount();
  vi.clearAllMocks();
  view.file = { id: "f1", path: PATH, result: { plane: BASE } };
  mocks.set.mockResolvedValue(RETURNED);
  mocks.clear.mockResolvedValue(RETURNED);
  mocks.sdss.mockResolvedValue(RETURNED);
  vi.spyOn(measurementLog, "append").mockImplementation((draft) => ({ ...draft, id: "id1", timestamp_utc: "2026-11-12T10:00:00.000Z" }));
});

afterEach(() => {
  hooks.mount();
  vi.restoreAllMocks();
});

describe("CompanionAssign", () => {
  it("shows where the error and the mask come from", () => {
    const plane = { ...IN_USE, pairing_notes: [NOTE], warnings: [WARNING] };
    const tree = render(plane);
    const root = tree as ReactElement<Props>;
    expect(root.type).toBe("section");
    expect(root.props["aria-label"]).toBe("Layers of the shown image");
    const text = textOf(tree);
    for (const part of ["Layers of the shown image", "EXTNAME VARIANCE, HDU 3: square root", "same file", "12 pixels without data", "EXTNAME MASK, HDU 2", NOTE]) {
      expect(text).toContain(part);
    }
    expect(textOf(row(tree, "Error"))).toContain("Error");
    expect(textOf(row(tree, "Mask"))).toContain("Mask");
    expect(elements(tree).find((e) => e.type === WarningList)?.props.warnings).toBe(plane.warnings);
    expect(text).not.toContain("Layers set by hand");
    expect(text).not.toContain("SDSS frame error");
    expect(elements(tree).find((e) => e.props.title === "EXTNAME VARIANCE, HDU 3: square root")).toBeDefined();
  });

  it("says why a layer is not used and still offers a file", () => {
    const tree = render({ ...BASE, err_unavailable: MISMATCH });
    const text = textOf(tree);
    expect(text).toContain(`not used: ${MISMATCH}`);
    expect(text).toContain("none found in this file or beside it");
    expect(named(tree, "Choose a file")).toHaveLength(2);
    expect(named(tree, "Switch off")).toHaveLength(0);
    expect(named(tree, "Automatic")).toHaveLength(0);
  });

  it("assigns the pending HDU with the chosen kind and puts the returned plane into the file row", async () => {
    const first = render(BASE, NOISE2);
    expect(textOf(first)).toContain("Use HDU 2 (NOISE2) as the error layer");
    const select = elements(first).find((e) => e.type === "select")!;
    expect(select.props["aria-label"]).toBe("Error layer kind");
    expect(elements(select).filter((e) => e.type === "option").map(textOf)).toEqual([
      "Automatic (from its name or cards)",
      "Sigma (as is)",
      "Variance (square root)",
      "Inverse variance (1 / square root)",
    ]);
    (select.props.onChange as (e: { target: { value: string } }) => void)({ target: { value: "variance" } });
    const chosen = render(BASE, NOISE2);
    expect(elements(chosen).find((e) => e.type === "select")!.props.value).toBe("variance");
    const sent = click(named(chosen, "Assign")[0]);
    expect(named(render(BASE, NOISE2), "Assign")[0].props.disabled).toBe(true);
    await sent;
    expect(mocks.set).toHaveBeenCalledWith(PATH, "err", "C:/d/a.fits#hdu=2", "variance");
    expect(mocks.apply).toHaveBeenCalledWith(view.file, RETURNED);
    expect(mocks.pending).toHaveBeenCalledWith(null);
    expect(named(render(BASE, NOISE2), "Assign")[0].props.disabled).toBe(false);

    hooks.mount();
    vi.clearAllMocks();
    mocks.set.mockResolvedValue(RETURNED);
    await click(named(render(BASE, NOISE2), "Assign")[0]);
    expect(mocks.set).toHaveBeenCalledWith(PATH, "err", "C:/d/a.fits#hdu=2", null);

    hooks.mount();
    vi.clearAllMocks();
    mocks.set.mockResolvedValue(RETURNED);
    const mask: PendingLayer = { role: "dq", source: "C:/d/a.fits#hdu=4", name: "HDU 4 (MSK)" };
    const dq = render(BASE, mask);
    expect(textOf(dq)).toContain("Use HDU 4 (MSK) as the mask");
    expect(elements(dq).find((e) => e.type === "select")).toBeUndefined();
    await click(named(dq, "Assign")[0]);
    expect(mocks.set).toHaveBeenCalledWith(PATH, "dq", "C:/d/a.fits#hdu=4", null);
  });

  it("keeps the form open with the sentence of the backend when the kind is needed", async () => {
    const refusal = "cannot tell whether noise.fits holds sigma, variance or inverse variance: choose the kind";
    mocks.set.mockRejectedValue(new Error(refusal));
    await click(named(render(BASE, NOISE2), "Assign")[0]);
    const after = render(BASE, NOISE2);
    expect(elements(after).find((e) => e.type === ErrorAlert)?.props.message).toBe(refusal);
    expect(textOf(after)).toContain("Use HDU 2 (NOISE2) as the error layer");
    expect(named(after, "Assign")[0].props.disabled).toBe(false);
    expect(mocks.pending).not.toHaveBeenCalled();
    expect(mocks.apply).not.toHaveBeenCalled();

    const cancel = named(after, "Cancel")[0];
    click(cancel);
    expect(mocks.pending).toHaveBeenCalledWith(null);
    const other: PendingLayer = { ...NOISE2, source: "C:/d/b.fits", name: "b.fits" };
    render(BASE, other);
    expect(elements(render(BASE, other)).find((e) => e.type === ErrorAlert)?.props.message ?? null).toBeNull();
  });

  it("switches a layer off and goes back to the automatic choice", async () => {
    const used = render(IN_USE);
    expect(named(used, "Automatic")).toHaveLength(0);
    await click(named(row(used, "Error"), "Switch off")[0]);
    expect(mocks.clear).toHaveBeenCalledWith(PATH, "err", true);
    expect(mocks.apply).toHaveBeenCalledWith(view.file, RETURNED);
    await click(named(row(render(IN_USE), "Mask"), "Switch off")[0]);
    expect(mocks.clear).toHaveBeenCalledWith(PATH, "dq", true);

    const off = { ...BASE, err_unavailable: "error layer switched off by the user", dq_unavailable: "mask layer switched off by the user" };
    const tree = render(off);
    expect(textOf(tree)).toContain(HAND_SET);
    expect(named(tree, "Switch off")).toHaveLength(0);
    await click(named(row(tree, "Error"), "Automatic")[0]);
    expect(mocks.clear).toHaveBeenCalledWith(PATH, "err", false);
    await click(named(row(render(off), "Mask"), "Automatic")[0]);
    expect(mocks.clear).toHaveBeenCalledWith(PATH, "dq", false);
  });

  it("takes a file from the dialog and waits for Assign", async () => {
    mocks.open.mockResolvedValue("C:/d/des-dr2_A_r_extn3.fits");
    await click(named(row(render(BASE), "Error"), "Choose a file")[0]);
    expect(mocks.open).toHaveBeenCalledWith({
      multiple: false,
      title: "Error layer of the shown image",
      filters: [{ name: "FITS", extensions: ["fits", "fit", "fts", "fz", "gz", "bz2"] }],
    });
    expect(mocks.pending).toHaveBeenCalledWith({ role: "err", source: "C:/d/des-dr2_A_r_extn3.fits", name: "des-dr2_A_r_extn3.fits" });
    expect(mocks.set).not.toHaveBeenCalled();

    vi.clearAllMocks();
    mocks.open.mockResolvedValue("C:/d/des-dr2_A_r_extn2.fits");
    await click(named(row(render(BASE), "Mask"), "Choose a file")[0]);
    expect(mocks.open).toHaveBeenCalledWith(expect.objectContaining({ title: "Mask of the shown image" }));
    expect(mocks.pending).toHaveBeenCalledWith({ role: "dq", source: "C:/d/des-dr2_A_r_extn2.fits", name: "des-dr2_A_r_extn2.fits" });

    vi.clearAllMocks();
    mocks.open.mockResolvedValue(null);
    await click(named(row(render(BASE), "Error"), "Choose a file")[0]);
    expect(mocks.open).toHaveBeenCalledTimes(1);
    expect(mocks.pending).not.toHaveBeenCalled();
    expect(mocks.set).not.toHaveBeenCalled();
  });

  it("offers the SDSS recipe on a frame only and sends the two typed numbers", async () => {
    const frame: PlaneInfo = { ...BASE, err_unavailable: SDSS_REASON, err_recipe: { kind: "sdss_frame", gain: null, dark_variance: null } };
    const first = render(frame);
    expect(textOf(first)).toContain("SDSS frame error");
    expect(textOf(first)).toContain("Neither number is in the file. Type the gain and the dark variance of this camera column and band.");
    expect(textOf(first)).toContain("Gain (e-/DN)");
    expect(textOf(first)).toContain("Dark variance (DN^2)");
    expect(field(first, "Gain (e-/DN)").props.value).toBe("");
    expect(field(first, "Dark variance (DN^2)").props.min).toBe(0);
    expect(named(first, "Rebuild the error layer")[0].props.disabled).toBe(true);
    (field(first, "Gain (e-/DN)").props.onChange as (text: string) => void)("4.76");
    (field(first, "Dark variance (DN^2)").props.onChange as (text: string) => void)("1.3225");
    const typed = render(frame);
    expect(field(typed, "Gain (e-/DN)").props.error ?? null).toBeNull();
    const rebuild = named(typed, "Rebuild the error layer")[0];
    expect(rebuild.props.disabled).toBe(false);
    await click(rebuild);
    expect(mocks.sdss).toHaveBeenCalledWith(PATH, 4.76, 1.3225);
    expect(mocks.apply).toHaveBeenCalledWith(view.file, RETURNED);

    (field(render(frame), "Gain (e-/DN)").props.onChange as (text: string) => void)("0");
    const zero = render(frame);
    expect(named(zero, "Rebuild the error layer")[0].props.disabled).toBe(true);
    expect(field(zero, "Gain (e-/DN)").props.error).toBe("Use a value above 0");
    (field(zero, "Gain (e-/DN)").props.onChange as (text: string) => void)("4.76");
    (field(zero, "Dark variance (DN^2)").props.onChange as (text: string) => void)("");
    expect(named(render(frame), "Rebuild the error layer")[0].props.disabled).toBe(true);

    hooks.mount();
    const typedBefore = render({ ...frame, err_recipe: { kind: "sdss_frame", gain: 4.76, dark_variance: 1.3225 } });
    expect(field(typedBefore, "Gain (e-/DN)").props.value).toBe("4.76");
    expect(field(typedBefore, "Dark variance (DN^2)").props.value).toBe("1.3225");

    hooks.mount();
    expect(textOf(render({ ...BASE, err_unavailable: SDSS_REASON }))).not.toContain("SDSS frame error");
  });

  it("only describes a plane that is itself a layer, and stays read-only for another file", () => {
    const shown = render({ ...BASE, layer: { role: "mask", basis: "EXTNAME MSK", assumed: false }, pairing_notes: [NOTE] });
    expect(textOf(shown)).toContain("The shown plane is a layer itself: mask (EXTNAME MSK). Layers are assigned to science images.");
    expect(textOf(shown)).toContain(NOTE);
    expect(textOf(shown)).not.toContain("Error");
    expect(buttons(shown)).toHaveLength(0);

    hooks.mount();
    view.file = { id: "f2", path: "C:/d/b.fits", result: { plane: BASE } };
    const elsewhere = buttons(render(IN_USE, NOISE2));
    expect(elsewhere.length).toBeGreaterThan(0);
    expect(elsewhere.every((b) => b.props.disabled === true)).toBe(true);

    hooks.mount();
    view.file = { id: "f1", path: PATH, result: null };
    expect(buttons(render(IN_USE)).every((b) => b.props.disabled === true)).toBe(true);
  });

  it("drops a result that arrives after the shown image changed", async () => {
    const late = deferred<PlaneInfo>();
    mocks.clear.mockReturnValue(late.promise);
    const sent = click(named(row(render(IN_USE), "Error"), "Switch off")[0]);
    render(IN_USE, null, "C:/d/a.fits#hdu=4");
    late.resolve(RETURNED);
    await sent;
    expect(mocks.apply).not.toHaveBeenCalled();
    expect(mocks.pending).not.toHaveBeenCalled();
    expect(named(row(render(IN_USE), "Error"), "Switch off")[0].props.disabled).toBe(false);
  });
});

describe("CompanionAssign measurement log", () => {
  const logged = () => vi.mocked(measurementLog.append).mock.calls.map(([draft]) => draft);

  it("logs a layer change once it has happened", async () => {
    await click(named(render(BASE, NOISE2), "Assign")[0]);
    expect(logged()).toHaveLength(1);
    const [assigned] = logged();
    expect(assigned.kind).toBe("layer_assignment");
    expect(assigned.file).toBe("a.fits");
    expect(assigned.params).toEqual({ role: "err", action: "assign", exclusion_mask: null });
    expect(assigned.values.err_origin).toBe("user");

    vi.clearAllMocks();
    mocks.clear.mockResolvedValue(RETURNED);
    await click(named(row(render(IN_USE), "Mask"), "Switch off")[0]);
    expect(logged().map((draft) => draft.params)).toEqual([{ role: "dq", action: "switch_off", exclusion_mask: null }]);

    vi.clearAllMocks();
    mocks.clear.mockResolvedValue(RETURNED);
    const off = { ...BASE, err_unavailable: "error layer switched off by the user" };
    await click(named(row(render(off), "Error"), "Automatic")[0]);
    expect(logged().map((draft) => draft.params)).toEqual([{ role: "err", action: "automatic", exclusion_mask: null }]);

    vi.clearAllMocks();
    mocks.sdss.mockResolvedValue(RETURNED);
    const frame: PlaneInfo = { ...BASE, err_unavailable: SDSS_REASON, err_recipe: { kind: "sdss_frame", gain: 4.76, dark_variance: 1.3225 } };
    hooks.mount();
    await click(named(render(frame), "Rebuild the error layer")[0]);
    expect(logged().map((draft) => draft.params)).toEqual([{ role: "err", action: "sdss_recipe", exclusion_mask: null }]);

    vi.clearAllMocks();
    hooks.mount();
    mocks.set.mockRejectedValue(new Error("cannot tell whether noise.fits holds sigma, variance or inverse variance: choose the kind"));
    await click(named(render(BASE, NOISE2), "Assign")[0]);
    expect(logged()).toEqual([]);
  });
});
