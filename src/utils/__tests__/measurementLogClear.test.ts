import { describe, expect, it } from "vitest";
import { LOG_SESSION_NOTE, MeasurementLogCore, clearConfirmText } from "../measurementLog";

describe("clearing the measurement log", () => {
  it("asks for the number of rows it will remove", () => {
    expect(clearConfirmText(12, 12)).toBe("Clear 12 rows?");
    expect(clearConfirmText(1, 1)).toBe("Clear 1 row?");
  });

  it("says every row goes when a filter shows only some of them", () => {
    expect(clearConfirmText(12, 3)).toBe("Clear all 12 rows, not only the 3 shown?");
  });

  it("tells the user the log lives only in this session", () => {
    expect(LOG_SESSION_NOTE).toBe("Kept for this session only; Save CSV to keep it.");
  });

  it("clear empties every file's rows", () => {
    const log = new MeasurementLogCore(() => new Date(0), () => "id");
    const draft = { kind: "pixel" as const, image: "loaded file", dq: "off" as const, unit: null, source: "s", params: {}, values: {}, notes: [] };
    log.append({ ...draft, file: "a.fits" });
    log.append({ ...draft, file: "b.fits" });
    log.clear();
    expect(log.getSnapshot()).toHaveLength(0);
  });
});
