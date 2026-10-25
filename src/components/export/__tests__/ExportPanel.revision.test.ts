import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactElement, type ReactNode } from "react";

const FILE = "C:\\out\\m16_wcs.fits";

const wcsInfoMock = vi.hoisted(() => vi.fn<(path: string) => Promise<unknown>>());
const cutoutMock = vi.hoisted(() => vi.fn<(...args: unknown[]) => Promise<unknown>>());
const regionDoc = vi.hoisted(() => ({ current: { regions: [] as unknown[], selectedId: null as string | null } }));

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../hooks/useRegionKey", () => ({ useRegionKey: () => FILE }));
vi.mock("../../../hooks/useRegionStore", () => ({ useRegionDoc: () => regionDoc.current }));
vi.mock("../../../services/astrometry", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/astrometry")>()),
  getWcsInfo: wcsInfoMock,
}));
vi.mock("../../../services/cutout", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/cutout")>()),
  exportCutout: cutoutMock,
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: async () => "C:\\out\\m16_cutout.fits" }));

import { hooks } from "../../../test/hookRuntime";
import ExportPanel from "../ExportPanel";
import { rememberSolvedWcs } from "../../../utils/solvedWcs";

const PROPS = { filePath: FILE, exportPath: FILE, stfParams: null, onExport: () => Promise.resolve() };

const render = () => hooks.render(ExportPanel as unknown as (props: typeof PROPS) => unknown, PROPS) as ReactNode;

type AnyProps = Record<string, unknown> & { children?: ReactNode };

function elements(node: ReactNode): ReactElement<AnyProps>[] {
  if (Array.isArray(node)) return node.flatMap(elements);
  if (!isValidElement<AnyProps>(node)) return [];
  return [node, ...elements(node.props.children)];
}

function textOf(node: ReactNode): string {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  return isValidElement<AnyProps>(node) ? textOf(node.props.children) : "";
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("ExportPanel after Write WCS rewrites the exported copy", () => {
  beforeEach(() => {
    hooks.mount();
    regionDoc.current = { regions: [], selectedId: null };
    wcsInfoMock.mockReset().mockResolvedValue({ pixel_scale_arcsec: 2.4 });
    cutoutMock.mockReset();
  });

  afterEach(() => hooks.mount());

  it("re-reads the pixel scale of the cutout when the file's WCS is rewritten", () => {
    render();
    expect(wcsInfoMock).toHaveBeenCalledTimes(1);
    render();
    expect(wcsInfoMock).toHaveBeenCalledTimes(1);
    rememberSolvedWcs("C:\\data\\m16.fits", FILE);
    expect(hooks.run.notified, "the panel follows the solved WCS store").toBeGreaterThan(0);
    render();
    expect(wcsInfoMock).toHaveBeenCalledTimes(2);
    expect(wcsInfoMock).toHaveBeenLastCalledWith(FILE);
  });

  it("keeps the saved cutout card when the file's WCS is rewritten, and drops it for another file", async () => {
    regionDoc.current = { regions: [{ id: "b1", shape: { shape: "box", x: 50, y: 40, width: 20, height: 10, angle: 0 } }], selectedId: "b1" };
    cutoutMock.mockResolvedValue({
      output_path: "C:\\out\\m16_cutout.fits",
      rect: { x0: 40, y0: 35, width: 20, height: 10 },
      fraction_on_image: 1,
      hdus: ["SCI"],
      ltv1: -40,
      ltv2: -35,
      rotated_box_used_bounds: false,
      warnings: [],
      elapsed_ms: 12,
    });
    const run = elements(render()).find((el) => el.props.label === "Export cutout (SCI/ERR/DQ)");
    await (run?.props.onClick as () => Promise<void>)();
    await flush();
    expect(textOf(render())).toContain("LTV1 -40, LTV2 -35");

    rememberSolvedWcs("C:\\data\\m16.fits", FILE);
    render();
    await flush();
    expect(textOf(render())).toContain("LTV1 -40, LTV2 -35");

    hooks.render(ExportPanel as unknown as (props: typeof PROPS) => unknown, { ...PROPS, filePath: "C:\\out\\other.fits" });
    expect(textOf(render())).not.toContain("LTV1 -40");
  });
});
