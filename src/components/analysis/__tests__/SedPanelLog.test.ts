import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactElement, ReactNode } from "react";

const dialog = vi.hoisted(() => ({ answer: "save" as string | null }));
const saved = vi.hoisted(() => [] as [string, string][]);

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  save: async (d: { defaultPath: string }) => (dialog.answer === null ? null : `C:/out/${d.defaultPath}`),
}));
vi.mock("@tauri-apps/plugin-fs", () => ({ writeTextFile: async (path: string, text: string) => void saved.push([path, text]) }));
vi.mock("../../../context/PreviewContext", () => ({
  useFileContext: () => ({ file: { id: "f1", path: "C:/data/cube.fits", name: "cube.fits" } }),
}));

import { hooks } from "../../../test/hookRuntime";
import { elementsOf, textOf } from "../../../test/elementTree";
import { measurementLog } from "../../../utils/measurementLog";
import { sedEntry } from "../../../utils/measurementLogForced";
import type { PhotometricPlane } from "../../../shared/types/cube";
import type { SedPoint, SedSource } from "../../../utils/sedAxis";
import SedPanel from "../SedPanel";

type Props = Record<string, unknown> & { children?: ReactNode };
type Element = ReactElement<Props>;
type Component = (props: Props) => unknown;

function plane(filter: string, pivot_um: number): PhotometricPlane {
  return {
    filter,
    dataset: "JWST",
    instrument: "NIRCam",
    pivot_um,
    pixscale_arcsec: 0.04,
    bunit_in: "Jy",
    fluxconv: 1,
    psf_fwhm_arcsec: null,
    kernel: "",
  };
}

const PLANES = [plane("F150W", 1.500744), plane("F277W", 2.761), plane("F444W", 4.404315)];
const PIXEL: SedSource = { spectrum: [3, 1, 2], err: [0.3, 0.1, 0.2], coord: { x: 2, y: 3 }, region: null };
const LIMIT_POINTS: SedPoint[] = [
  { index: 0, filter: "g", pivotUm: 0.4808, value: 2, err: 0.1 },
  { index: 1, filter: "r", pivotUm: 0.6418, value: 0.3, err: null, limit: true },
];
const POINTS_PROPS = { points: LIMIT_POINTS, unit: "uJy", label: "s0, aperture fluxes", fileNames: ["g.fits", "r.fits"] };
const CUBE_PROPS = { planes: PLANES, bunit: "MJy/sr", source: PIXEL };

const clipboard = vi.fn<(text: string) => Promise<void>>();
const append = vi.spyOn(measurementLog, "append");
const tick = async () => {
  for (let i = 0; i < 5; i++) await new Promise((resolve) => setTimeout(resolve, 0));
};

function viewOf(props: Props): Element {
  const panel = (SedPanel as unknown as { type: Component }).type;
  hooks.mount();
  const form = hooks.render(panel, props) as Element;
  hooks.mount();
  const view = hooks.render(form.type as Component, form.props) as Element;
  hooks.mount();
  return view;
}

const show = (view: Element) => elementsOf<Props>(hooks.render(view.type as Component, view.props)) as Element[];
const buttonOf = (shown: Element[], text: string) => shown.find((e) => e.type === "button" && textOf(e.props.children) === text);
const click = (element: Element | undefined) => (element!.props.onClick as () => void)();

describe("SedPanel measurement log rows", () => {
  beforeEach(() => {
    hooks.mount();
    dialog.answer = "save";
    saved.length = 0;
    append.mockClear();
    clipboard.mockReset().mockResolvedValue(undefined);
    vi.stubGlobal("navigator", { clipboard: { writeText: clipboard } });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    hooks.mount();
  });

  it("copying the SED of a cube pixel logs its points", async () => {
    const view = viewOf(CUBE_PROPS);
    click(buttonOf(show(view), "Copy CSV"));
    await tick();
    expect(clipboard).toHaveBeenCalledTimes(1);
    expect(append).toHaveBeenCalledTimes(1);
    const draft = append.mock.calls[0][0];
    expect(draft.kind).toBe("sed");
    expect(draft.file).toBe("cube.fits");
    expect(draft.image).toBe("loaded cube");
    expect(draft.unit).toBe("MJy/sr");
    expect(draft.source).toBe('{"kind":"sed","label":"pixel (2, 3)"}');
    expect(draft.params.saved_path).toBeNull();
    expect(draft.params.n_points).toBe(3);
    expect(draft.values.flux_F150W).toBe(3);
    expect(draft.values.flux_F277W).toBe(1);
    expect(draft.values.flux_err_F444W).toBe(0.2);
  });

  it("saving the SED of a multi-band source logs where it went", async () => {
    const view = viewOf(POINTS_PROPS);
    click(buttonOf(show(view), "Save CSV"));
    await tick();
    expect(saved).toHaveLength(1);
    expect(append).toHaveBeenCalledTimes(1);
    const draft = append.mock.calls[0][0];
    expect(draft).toEqual(
      sedEntry({ file: "g.fits; r.fits", image: "original", label: "s0, aperture fluxes", unit: "uJy", points: LIMIT_POINTS, savedPath: "C:/out/sed.csv" }),
    );
    expect(draft.params.saved_path).toBe("C:/out/sed.csv");
    expect(draft.values.limit_r).toBe(true);

    dialog.answer = null;
    click(buttonOf(show(view), "Save CSV"));
    await tick();
    expect(saved).toHaveLength(1);
    expect(append).toHaveBeenCalledTimes(1);
  });

  it("a failed export logs nothing", async () => {
    clipboard.mockRejectedValueOnce(new Error("denied"));
    const view = viewOf(POINTS_PROPS);
    click(buttonOf(show(view), "Copy CSV"));
    await tick();
    expect(append).not.toHaveBeenCalled();
    const alert = show(view).find((e) => e.props.role === "alert");
    expect(textOf(alert?.props.children)).toBe("SED export failed: denied");
  });
});
