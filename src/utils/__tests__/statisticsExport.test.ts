import { describe, expect, it } from "vitest";
import { statisticsClipboardCsv, type StatisticsCsvInput } from "../statisticsExport";
import { APP_VERSION } from "../constants";

const INPUT: StatisticsCsvInput = {
  table: "statistic,K\ncount,5\nmean,22",
  file: "x_i2d.fits",
  region: "box region",
  bunit: "MJy/sr",
  displayUnit: "raw",
  dqExcluded: 812,
  dqGap: null,
  noiseMethod: "k-sigma-mrs",
  exportedUtc: "2026-11-12T10:00:00.000Z",
};

const linesOf = (csv: string) => csv.split("\n");
const provenanceOf = (csv: string) => linesOf(csv).filter((line) => line.startsWith("#"));
const bodyOf = (csv: string) => linesOf(csv).filter((line) => !line.startsWith("#") && line !== "");

describe("statisticsClipboardCsv", () => {
  it("the copied statistics say which image, region and unit they describe", () => {
    const csv = statisticsClipboardCsv(INPUT);
    expect(provenanceOf(csv)).toEqual([
      `# software: AstroBurst ${APP_VERSION}`,
      "# exported_utc: 2026-11-12T10:00:00.000Z",
      "# file: x_i2d.fits",
      "# region: box region",
      "# dq_excluded: 812",
      "# noise_method: k-sigma-mrs",
      "# bunit: MJy/sr",
      "# display_unit: raw",
    ]);
    expect(linesOf(csv).some((line) => line.startsWith("# unit "))).toBe(false);
    expect(bodyOf(csv)[0]).toBe("statistic,K");
    expect(bodyOf(csv)).toEqual(["statistic,K", "count,5", "mean,22"]);
    expect(linesOf(csv)).not.toContain("unit,raw");
  });

  it("says when the exclusion was not applied and leaves a missing file or unit empty", () => {
    const gap = provenanceOf(
      statisticsClipboardCsv({ ...INPUT, dqExcluded: null, dqGap: "this image has no DQ plane", displayUnit: "normalized", noiseMethod: null, file: null }),
    );
    expect(gap).toContain("# file:");
    expect(gap).toContain("# dq_exclusion: requested but not applied: this image has no DQ plane");
    expect(gap).toContain("# display_unit: normalized");
    expect(gap.some((line) => line.startsWith("# dq_excluded") || line.startsWith("# noise_method") || line.startsWith("# unit"))).toBe(false);

    const noUnit = provenanceOf(statisticsClipboardCsv({ ...INPUT, displayUnit: "raw", bunit: null }));
    expect(noUnit).toContain("# bunit:");
    expect(noUnit.some((line) => line.startsWith("# unit"))).toBe(false);
  });
});
