import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { hooks } from "../../../test/hookRuntime";
import type { PixelProbeResult } from "../../../shared/types/analysis";

const mocks = vi.hoisted(() => ({ probe: vi.fn() }));
vi.mock("react", async (original) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await original()));
vi.mock("../../../services/display", () => ({ probePixel: mocks.probe }));

import PixelReadout from "../PixelReadout";

const props = { filePath: "a.fits", mouseX: 4, mouseY: 5 };
const component = (PixelReadout as unknown as { type: (p: typeof props) => ReactNode }).type;
const markup = () => renderToStaticMarkup(hooks.render(component, props) as ReactNode);

function probe(overrides: Partial<PixelProbeResult>): PixelProbeResult {
  return {
    x: 4,
    y: 5,
    box: 5,
    value: 12,
    unit: "nJy",
    neighborhood: { min: 12, max: 12, mean: 12, median: 12, n_pixels: 25, n_nan: 0 },
    dq: null,
    err: null,
    wavelength: null,
    ...overrides,
  };
}

async function shown(result: PixelProbeResult): Promise<string> {
  mocks.probe.mockResolvedValue(result);
  markup();
  await vi.advanceTimersByTimeAsync(40);
  return markup();
}

beforeEach(() => {
  hooks.mount();
  vi.useFakeTimers();
  vi.resetAllMocks();
});

afterEach(() => {
  hooks.mount();
  vi.useRealTimers();
});

describe("PixelReadout error origin and flag set", () => {
  it("shows where a converted error comes from", async () => {
    const html = await shown(probe({ err: { value: 5, unit: "nJy", source: "EXTNAME VARIANCE, HDU 3: square root" } }));
    expect(html).toContain("± 5 nJy");
    expect(html).toContain('title="Error layer: EXTNAME VARIANCE, HDU 3: square root"');
    expect(html).toContain("± from EXTNAME VARIANCE, HDU 3: square root");
  });

  it("keeps the readout of a pipeline product as it was, with the origin in the tooltip only", async () => {
    const pipeline = await shown(probe({ err: { value: 5, unit: "nJy", source: "EXTNAME ERR, HDU 2: as is" } }));
    expect(pipeline).toContain('title="Error layer: EXTNAME ERR, HDU 2: as is"');
    expect(pipeline).not.toContain("± from");
    hooks.mount();
    const bare = await shown(probe({ err: { value: 5, unit: "nJy" } }));
    expect(bare).toContain("± 5 nJy");
    expect(bare).not.toContain("Error layer:");
    hooks.mount();
    const none = await shown(probe({ err: null }));
    expect(none).toContain("12 nJy");
    expect(none).not.toContain("±");
  });

  it("names the flag set on the DQ line", async () => {
    const dq = { bits: 34, value: 34, names: ["SAT", "DETECTED"], table: "unknown" as const, text: "34: SAT | DETECTED" };
    const named = await shown(probe({ dq: { ...dq, source: "MP_ cards (afw mask planes)" } }));
    expect(named).toContain('title="MP_ cards (afw mask planes)"><span>DQ = 34: SAT | DETECTED</span>');
    hooks.mount();
    const plain = await shown(probe({ dq }));
    expect(plain).toContain('title="unknown"><span>DQ = 34: SAT | DETECTED</span>');
  });
});
