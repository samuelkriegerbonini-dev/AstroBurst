import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { SaguiRegionalBand, SaguiResult } from "../../../shared/types/sagui";
import SaguiViewer from "../SaguiViewer";

const band = (filter: string, pivot_um: number, flux: number, error: number | null = null, error_correlated?: number | null): SaguiRegionalBand => ({ filter, pivot_um, flux, error, ...(error_correlated === undefined ? {} : { error_correlated }) });
const render = (bands: SaguiRegionalBand[]) => {
  const result: SaguiResult = { run_id: "viewer", unit: "Jy", output_dir: "/out/sagui-viewer", labels_path: "/out/sagui-viewer/labels.fits", mask_path: "/out/sagui-viewer/support.fits", table_path: "/out/sagui-viewer/regions.csv", manifest_path: "/out/sagui-viewer/manifest.json", width: 1, height: 1, labels: [1], warnings: [], regions: [{ region: 1, n_pixels: 1, bands }] };
  return renderToStaticMarkup(createElement(SaguiViewer, { result }));
};
const pointHeights = (html: string) => [...html.matchAll(/<circle [^>]*cy="([^"]+)"/g)].map((match) => Number(match[1]));
const svgOf = (html: string) => /<svg[\s\S]*<\/svg>/.exec(html)?.[0] ?? "";

describe("SAGUI region list", () => {
  it("lists the regions largest first with components and minimum SNR, and offers to save the selected SED (proposals 20, 22)", () => {
    const bands = [band("F200W", 2, 1, 0.1)];
    const result: SaguiResult = { run_id: "list", unit: "Jy", output_dir: "/out", labels_path: "l.fits", mask_path: "m.fits", table_path: "r.csv", manifest_path: "manifest.json", width: 2, height: 1, labels: [1, 2], warnings: [], regions: [{ region: 1, n_pixels: 5, n_components: 1, min_snr: 12.345, bands }, { region: 2, n_pixels: 40, n_components: 3, min_snr: null, min_snr_lower_bound: 2.5, bands }] };
    const html = renderToStaticMarkup(createElement(SaguiViewer, { result, onSaveRegion: () => undefined }));
    expect(html).toMatch(/<button[^>]*aria-pressed="false"[^>]*>Region 2<\/button><\/td><td[^>]*>40<\/td><td[^>]*>3<\/td><td[^>]*>≥ 2.5<\/td>[\s\S]*<button[^>]*aria-pressed="true"[^>]*>Region 1<\/button><\/td><td[^>]*>5<\/td><td[^>]*>1<\/td><td[^>]*>12.3<\/td>/);
    expect(html).toContain("Save the SED of region 1 as CSV");
    expect(renderToStaticMarkup(createElement(SaguiViewer, { result }))).not.toContain("as CSV");
  });
});

describe("SAGUI regional SED plot", () => {
  it("uses a log flux axis when every flux is positive and a linear one when any flux is zero or negative (SCIENCE-22)", () => {
    const html = render([band("F090W", 0.902, 2e-6), band("F200W", 1.99, 2e-5), band("F444W", 4.42, 3e-4)]);
    expect(html).toContain("Summed flux (Jy), log scale");
    expect(html).toContain(">1e-5</text>");
    expect(html).toContain(">1e-4</text>");
    const [faint, middle, bright] = pointHeights(html);
    expect(faint - middle).toBeGreaterThan(40);
    expect(middle - bright).toBeGreaterThan(40);
    for (const flux of [0, -2]) {
      const linear = render([band("F200W", 2, flux, 3), band("F444W", 4.4, 8)]);
      expect(linear).toContain("Summed flux (Jy)");
      expect(linear).not.toContain("log scale");
      expect(pointHeights(linear)).toHaveLength(2);
      expect(pointHeights(linear).every(Number.isFinite)).toBe(true);
    }
  });

  it("draws a lower error bar that crosses zero down to the axis on the log scale", () => {
    const html = render([band("F200W", 2, 1e-5, 2e-5), band("F444W", 4.4, 1e-3, 1e-4)]);
    expect(html).toContain("log scale");
    expect(html).not.toContain("NaN");
    expect(html).not.toContain("Infinity");
  });

  it("prints a pivot label only where it does not overlap the previous one (SCIENCE-22)", () => {
    const html = render([band("F090W", 0.902, 1), band("F200W", 1.99, 2), band("F210M", 2.09, 2), band("F444W", 4.42, 3)]);
    const labels = [...svgOf(html).matchAll(/>([0-9.]+)<\/text>/g)].map((match) => match[1]);
    expect(labels).toContain("0.902");
    expect(labels).toContain("4.42");
    expect(labels.filter((label) => label === "1.99" || label === "2.09")).toHaveLength(1);
    expect(pointHeights(html)).toHaveLength(4);
  });

  it("paints axes, grid, labels and series with theme tokens instead of fixed colors (SCIENCE-30)", () => {
    for (const bands of [[band("F200W", 2, 1, 0.1), band("F444W", 4.4, 2, 0.1)], [band("F200W", 2, -1, 0.1), band("F444W", 4.4, 2, 0.1)]]) {
      const svg = svgOf(render(bands));
      expect(svg).not.toMatch(/#[0-9a-f]{3,8}\b/i);
      for (const token of ["--ab-cyan", "--ab-text-1", "--ab-text-2", "--ab-border-strong"]) expect(svg).toContain(`var(${token})`);
    }
  });

  it("shows the correlated uncertainty beside the independent-pixel one and says what it includes", () => {
    const html = render([band("F200W", 2, 1e-3, 1e-5, 2.5e-5), band("F356W", 3.56, 1.5e-3, 1.5e-5, null), band("F444W", 4.4, 2e-3, 2e-5)]);
    expect(html).toContain(">Uncertainty</th><th class=\"text-right\">Correlated</th>");
    expect(html).toContain(">2.5000e-5</td>");
    expect(html.match(/>unavailable<\/td>/g)).toHaveLength(2);
    expect(html).toContain("Correlated uncertainty includes the noise correlation from resampling and PSF matching; Uncertainty assumes independent pixels.");
  });
});
