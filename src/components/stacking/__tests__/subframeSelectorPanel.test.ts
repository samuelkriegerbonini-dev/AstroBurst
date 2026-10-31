import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createElement, isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../services/analysis", () => ({ analyzeSubframes: vi.fn() }));

import SubframeSelectorPanel, { SubframeResultsView, SubframeStatusButton, ThSort } from "../SubframeSelectorPanel";
import { analyzeSubframes, type SubframeMetrics } from "../../../services/analysis";
import { DEFAULT_SUBFRAME_SETTINGS, subframeSettingsChanged } from "../../../utils/subframeSettings";
import { hooks } from "../../../test/hookRuntime";

describe("Subframe Selector frame list", () => {
  const LEFT_OUT = "8 frames left out by IMAGETYP: 4 darks, 4 flats";

  it("names the frames left out of the analysis by IMAGETYP under the file count", () => {
    const html = renderToStaticMarkup(createElement(SubframeSelectorPanel, { files: ["C:/s/light_1.fits", "C:/s/light_2.fits"], leftOutLine: LEFT_OUT }));
    expect(html).toMatch(/>2 files available for analysis\.<\/div><p[^>]*data-testid="subframe-left-out"[^>]*>8 frames left out by IMAGETYP: 4 darks, 4 flats<\/p>/);
  });

  it("still names them when only calibration frames are loaded", () => {
    const html = renderToStaticMarkup(createElement(SubframeSelectorPanel, { files: [], leftOutLine: LEFT_OUT }));
    expect(html).toContain("Load FITS files to analyze subframe quality.");
    expect(html).toContain(`data-testid="subframe-left-out"`);
    expect(html).toContain(LEFT_OUT);
  });

  it("shows no line when nothing was left out", () => {
    const html = renderToStaticMarkup(createElement(SubframeSelectorPanel, { files: ["C:/s/light_1.fits"] }));
    expect(html).toContain("1 file available for analysis.");
    expect(html).not.toContain("subframe-left-out");
  });
});

describe("Subframe Selector table after Analyze", () => {
  function metrics(name: string, fwhm: number, accepted: boolean): SubframeMetrics {
    return {
      file_path: `C:/sx03/${name}`,
      file_name: name,
      star_count: 80,
      median_fwhm: fwhm,
      median_eccentricity: 0.2,
      median_snr: 40,
      background_median: 100,
      background_sigma: 4,
      noise_ratio: 0.04,
      noise_sigma: 0.001234,
      noise_fraction: 0.9,
      weight: accepted ? 0.8 : 0,
      accepted,
    };
  }

  const ROWS = [metrics("frame_0001.fits", 2.01, true), metrics("frame_0005.fits", 6.05, true), metrics("frame_0009.fits", 9.2, false)];
  const ANALYSED = DEFAULT_SUBFRAME_SETTINGS;

  function view(stale: boolean, staleNote?: string): string {
    return renderToStaticMarkup(createElement(SubframeResultsView, {
      elapsedMs: 120,
      rows: ROWS,
      thresholds: ANALYSED,
      stale,
      staleNote,
      overrides: {},
      sortBy: "weight",
      sortAsc: false,
      onSort: vi.fn(),
      onToggle: vi.fn(),
      onApply: vi.fn(),
    }));
  }

  function applyTag(html: string): string {
    const tag = /<button[^>]*>Apply Selection<\/button>/.exec(html)?.[0];
    if (!tag) throw new Error("no Apply Selection button");
    return tag;
  }

  function fwhmCells(html: string): [string, boolean][] {
    return [...html.matchAll(/<td class="([^"]*)">(\d+\.\d\d)<\/td>/g)].map((m) => [m[2], m[1].includes("text-red-400")]);
  }

  it("after Max FWHM changes, Apply is disabled and the stale line shows", () => {
    const stale = subframeSettingsChanged(ANALYSED, { ...ANALYSED, maxFwhm: 5.0 });
    const html = view(stale);
    expect(applyTag(html)).toContain('disabled=""');
    const line = /<div id="([^"]+)" role="status"[^>]*>([^<]*)<\/div>/.exec(html);
    expect(line?.[2]).toBe("Settings changed since the analysis: Analyze again");
    expect(applyTag(html)).toContain(`aria-describedby="${line?.[1]}"`);
  });

  it("after a scoring weight changes, the table is stale too", () => {
    const html = view(subframeSettingsChanged(ANALYSED, { ...ANALYSED, noiseWeight: 1.0 }));
    expect(applyTag(html)).toContain('disabled=""');
    expect(html).toContain("Settings changed since the analysis: Analyze again");
  });

  it("colours the cells against the analysed thresholds, not the moved slider", () => {
    expect(fwhmCells(view(true))).toEqual([["2.01", false], ["6.05", false], ["9.20", true]]);
  });

  it("keeps Apply enabled and the status line empty while the settings match the analysis", () => {
    const html = view(subframeSettingsChanged(ANALYSED, { ...ANALYSED }));
    expect(applyTag(html)).not.toContain('disabled=""');
    expect(html).toMatch(/<div id="[^"]+" role="status"[^>]*><\/div>/);
    expect(html).not.toContain("Settings changed since the analysis");
  });

  it("shows the cause the panel names in the stale line", () => {
    const html = view(true, "Frames changed since the analysis: Analyze again");
    expect(applyTag(html)).toContain('disabled=""');
    expect(/<div id="[^"]+" role="status"[^>]*>([^<]*)<\/div>/.exec(html)?.[1]).toBe("Frames changed since the analysis: Analyze again");
  });
});

describe("Subframe Selector after the analysed frames change", () => {
  type AnyProps = Record<string, unknown> & { children?: ReactNode };
  const A = "C:/sx03/frame_0001.fits";
  const B = "C:/sx03/frame_0002.fits";
  const C = "C:/sx03/frame_0003.fits";
  const panel = SubframeSelectorPanel as (props: Parameters<typeof SubframeSelectorPanel>[0]) => ReactNode;

  function nodes(node: ReactNode): ReactElement<AnyProps>[] {
    if (Array.isArray(node)) return node.flatMap(nodes);
    if (!isValidElement<AnyProps>(node)) return [];
    return [node, ...nodes(node.props.children)];
  }

  const resultsView = (tree: ReactNode) => nodes(tree).find((el) => el.type === SubframeResultsView);

  beforeEach(() => {
    hooks.mount();
    vi.mocked(analyzeSubframes).mockReset().mockResolvedValue({ subframes: [], total: 2, accepted: 2, rejected: 0, elapsed_ms: 50 });
  });

  afterEach(() => hooks.mount());

  async function analysed(files: string[]): Promise<{ props: Parameters<typeof SubframeSelectorPanel>[0]; view: () => ReactNode }> {
    const props = { files, onSelectionChange: vi.fn() };
    const view = () => hooks.render(panel, props) as ReactNode;
    void (nodes(view()).find((el) => el.props.runningLabel === "Analyzing...")?.props.onClick as () => Promise<void>)();
    await vi.waitFor(() => expect(resultsView(view())).toBeDefined());
    return { props, view };
  }

  it("goes stale when a frame is added or removed after Analyze, and says the frames changed", async () => {
    const { props, view } = await analysed([A, B]);
    expect(resultsView(view())?.props.stale).toBe(false);
    props.files = [A, B, C];
    expect(resultsView(view())?.props).toMatchObject({ stale: true, staleNote: "Frames changed since the analysis: Analyze again" });
    props.files = [A];
    expect(resultsView(view())?.props.stale).toBe(true);
  });

  it("stays fresh when the same frames come back in another order", async () => {
    const { props, view } = await analysed([A, B]);
    props.files = [B, A];
    expect(resultsView(view())?.props.stale).toBe(false);
  });
});

type ClickProps = { onClick: (e: { stopPropagation: () => void }) => void };

function sortHeader(col: "file_name" | "weight", current: "file_name" | "weight", asc: boolean, onClick = vi.fn()): string {
  const th = createElement(ThSort, { label: col === "weight" ? "Weight" : "File", col, current, asc, onClick });
  return renderToStaticMarkup(createElement("table", null, createElement("thead", null, createElement("tr", null, th))));
}

function buttonIn(node: unknown): ReactElement<ClickProps> | null {
  if (!node || typeof node !== "object" || !("type" in node)) return null;
  const el = node as ReactElement<{ children?: unknown }>;
  if (el.type === "button") return el as unknown as ReactElement<ClickProps>;
  for (const child of [el.props.children].flat()) {
    const found = buttonIn(child);
    if (found) return found;
  }
  return null;
}

function findButton(node: unknown): ReactElement<ClickProps> {
  const button = buttonIn(node);
  if (!button) throw new Error("no button rendered");
  return button;
}

describe("Subframe status cell", () => {
  it("is a toggle button that announces an accepted frame", () => {
    const html = renderToStaticMarkup(createElement(SubframeStatusButton, { name: "light_001.fits", accepted: true, onToggle: () => {} }));
    expect(html).toMatch(/^<button type="button"/);
    expect(html).toContain('aria-pressed="true"');
    expect(html).toContain('aria-label="Frame light_001.fits: accepted, press to reject"');
  });

  it("announces a rejected frame and the action that accepts it", () => {
    const html = renderToStaticMarkup(createElement(SubframeStatusButton, { name: "light_002.fits", accepted: false, onToggle: () => {} }));
    expect(html).toContain('aria-pressed="false"');
    expect(html).toContain('aria-label="Frame light_002.fits: rejected, press to accept"');
  });

  it("toggles once per activation without also firing the row click", () => {
    const onToggle = vi.fn();
    const stopPropagation = vi.fn();
    const button = findButton(SubframeStatusButton({ name: "light_001.fits", accepted: true, onToggle }));
    button.props.onClick({ stopPropagation });
    expect(onToggle).toHaveBeenCalledTimes(1);
    expect(stopPropagation).toHaveBeenCalledTimes(1);
  });
});

describe("Subframe sort headers", () => {
  it("mark the sorted column with aria-sort and wrap the label in a button", () => {
    expect(sortHeader("weight", "weight", false)).toMatch(/<th aria-sort="descending"[^>]*><button type="button"[^>]*>Weight/);
    expect(sortHeader("weight", "weight", true)).toContain('aria-sort="ascending"');
  });

  it("mark the other sortable columns as unsorted", () => {
    expect(sortHeader("file_name", "weight", false)).toMatch(/<th aria-sort="none"[^>]*><button type="button"[^>]*>File<\/button><\/th>/);
  });

  it("keep the direction arrow out of the accessible name", () => {
    expect(sortHeader("weight", "weight", false)).toMatch(/<span aria-hidden="true"[^>]*>▼<\/span>/);
  });

  it("sort by the column when the button is activated", () => {
    const onClick = vi.fn();
    const button = findButton(ThSort({ label: "File", col: "file_name", current: "weight", asc: false, onClick }));
    button.props.onClick({ stopPropagation: () => {} });
    expect(onClick).toHaveBeenCalledWith("file_name");
  });
});
