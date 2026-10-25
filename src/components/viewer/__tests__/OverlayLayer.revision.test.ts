import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const FILE = "C:\\data\\m16.fits";
const WCS_COPY = "C:\\out\\m16_wcs.fits";

const gridLinesMock = vi.hoisted(() => vi.fn<(...args: unknown[]) => Promise<unknown>>());
const wcsInfoMock = vi.hoisted(() => vi.fn<(path: string) => Promise<unknown>>());

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../context/PreviewContext", () => ({
  useDisplayContext: () => ({ display: { grid: true, gridFrame: "icrs", gridDensity: 3, compass: true } }),
  useDisplayedImage: () => ({ path: WCS_COPY, dimensions: [2000, 1500], isProcessed: true, label: null, previewOnly: false, previewUrl: null }),
}));
vi.mock("../../../hooks/useRegionKey", () => ({ useRegionKey: () => FILE }));
vi.mock("../../../hooks/useOverlayStore", () => ({ useOverlayDoc: () => ({ layers: [] }) }));
vi.mock("../../../services/astrometry", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/astrometry")>()),
  gridLines: gridLinesMock,
  getWcsInfo: wcsInfoMock,
}));

import { hooks } from "../../../test/hookRuntime";
import OverlayLayer from "../OverlayLayer";
import { rememberSolvedWcs } from "../../../utils/solvedWcs";

type Component = (props: Record<string, unknown>) => unknown;

const PROPS = {
  containerRef: { current: null },
  transform: { scale: 1, x: 0, y: 0 },
  renderW: 2000,
  renderH: 1500,
  fitsW: 2000,
  fitsH: 1500,
  enabled: true,
};

function render(): void {
  hooks.render((OverlayLayer as unknown as { type: Component }).type, PROPS);
}

const GRID = { lines: [], labels: [], frame: "icrs" };
const INFO = { north_vec: [0, -1], east_vec: [-1, 0], pixel_scale_arcsec: 2.4, naxis1: 2000, naxis2: 1500 };

describe("viewer overlay after Write WCS rewrites the displayed copy", () => {
  beforeEach(() => {
    hooks.mount();
    gridLinesMock.mockReset().mockResolvedValue(GRID);
    wcsInfoMock.mockReset().mockResolvedValue(INFO);
  });

  afterEach(() => hooks.mount());

  it("re-requests the grid and the compass of the displayed file when its WCS is rewritten in place", () => {
    render();
    expect(gridLinesMock).toHaveBeenCalledTimes(1);
    expect(wcsInfoMock).toHaveBeenCalledTimes(1);
    render();
    expect(gridLinesMock).toHaveBeenCalledTimes(1);
    rememberSolvedWcs(FILE, WCS_COPY);
    expect(hooks.run.notified, "the overlay follows the solved WCS store").toBeGreaterThan(0);
    render();
    expect(gridLinesMock).toHaveBeenCalledTimes(2);
    expect(gridLinesMock.mock.calls[1]).toEqual([WCS_COPY, "icrs", 3]);
    expect(wcsInfoMock).toHaveBeenCalledTimes(2);
  });

  it("ignores a WCS written for another file", () => {
    render();
    rememberSolvedWcs("C:\\data\\other.fits", "C:\\out\\other_wcs.fits");
    render();
    expect(gridLinesMock).toHaveBeenCalledTimes(1);
  });
});
