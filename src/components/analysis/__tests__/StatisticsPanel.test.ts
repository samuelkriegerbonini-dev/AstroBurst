import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactNode } from "react";
import type { ChannelStatistics, StatisticsResult } from "../../../shared/types/statistics";

const backend = vi.hoisted(() => ({ compute: vi.fn<(...args: unknown[]) => Promise<unknown>>(), clipboard: vi.fn() }));

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../context/PreviewContext", () => ({ useDqContext: () => ({ excludeDq: false }) }));
vi.mock("../../../hooks/useMeasurementLog", () => ({ useMeasurementProvenance: () => ({ file: "502nmos.fits", image: "original" }) }));
vi.mock("../../../hooks/useRegionKey", () => ({ useRegionKey: () => "C:/data/502nmos.fits" }));
vi.mock("../../../hooks/useRegionStore", () => ({ useRegionDoc: () => ({ regions: [], selectedId: null }) }));
vi.mock("../../../services/statistics", () => ({ computeStatistics: backend.compute, computeStatisticsComposite: backend.compute }));

import { hooks } from "../../../test/hookRuntime";
import StatisticsPanel from "../StatisticsPanel";
import { STATISTIC_ROWS, formatStatistic, statisticValue } from "../../../utils/statisticsUnits";

const STATS: ChannelStatistics = {
  count: 5, total: 6, fraction: 5 / 6, mean: 22, median: 3, avg_dev: 20.2, mad: 1.25, bwmv_sqrt: 1.5, min: 1, max: 100,
  sum: 110, variance: 1911.5, std_dev: Math.sqrt(1911.5), nan_count: 1, padding: 0, excluded: 0,
};

const RESULT: StatisticsResult = { statistics: STATS, noise: null, noise_note: null, data_min: 1, data_max: 100, unit: null, masked: false, dq_excluded: null, elapsed_ms: 3 };

type Props = Record<string, unknown> & { children?: ReactNode };

function elements(node: ReactNode): Props[] {
  if (Array.isArray(node)) return node.flatMap(elements);
  if (!isValidElement<Props>(node)) return [];
  return [node.props, ...elements(node.props.children)];
}

function textOf(node: ReactNode): string {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  return isValidElement<Props>(node) ? textOf(node.props.children) : "";
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));
const PROPS = { filePath: "C:/data/502nmos.fits", composite: false, rgbPath: null, measureKey: null };
type Panel = (props: typeof PROPS) => unknown;
const show = () => hooks.render((StatisticsPanel as unknown as { type: Panel }).type, PROPS) as ReactNode;

async function computed(): Promise<ReactNode> {
  show();
  await flush();
  return show();
}

describe("StatisticsPanel table (UX-35)", () => {
  beforeEach(() => {
    hooks.mount();
    backend.compute.mockReset().mockResolvedValue(RESULT);
    vi.stubGlobal("navigator", { clipboard: { writeText: backend.clipboard } });
    vi.stubGlobal("window", { setTimeout, clearTimeout });
  });

  afterEach(() => {
    hooks.mount();
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("tags each row with its key, gives the label cell the row title and reads every cell through statisticValue", async () => {
    const rows = elements(await computed()).filter((p) => typeof p["data-statistic"] === "string");
    expect(rows.map((row) => row["data-statistic"])).toEqual(STATISTIC_ROWS.map((row) => row.key));
    for (const [i, row] of rows.entries()) {
      const spec = STATISTIC_ROWS[i];
      const [label, value] = elements(row.children).filter((p) => p.children !== undefined && !Array.isArray(p.children));
      expect(label.title, spec.key).toBe(spec.title);
      expect(textOf(label.children as ReactNode), spec.key).toBe(spec.label);
      expect(textOf(value.children as ReactNode), spec.key).toBe(formatStatistic(statisticValue(STATS, spec.key), spec.kind, "raw"));
    }
    const sigma = rows.find((row) => row["data-statistic"] === "sigma_mad")!;
    expect(textOf(sigma.children as ReactNode)).toContain(formatStatistic(1.4826 * 1.25, "scale", "raw"));
  });

  it("keeps the Copied tick for its full time after a second copy", async () => {
    await computed();
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    vi.stubGlobal("window", { setTimeout, clearTimeout });
    const copy = () => (elements(show()).find((p) => p.title === "Copy the table as CSV")!.onClick as () => void)();
    const ticked = () => textOf(show()).includes("Copied");
    copy();
    expect(ticked()).toBe(true);
    vi.advanceTimersByTime(1_000);
    copy();
    vi.advanceTimersByTime(1_000);
    expect(ticked(), "1 s after the second copy").toBe(true);
    vi.advanceTimersByTime(600);
    expect(ticked()).toBe(false);
    expect(backend.clipboard).toHaveBeenCalledTimes(2);
  });
});
