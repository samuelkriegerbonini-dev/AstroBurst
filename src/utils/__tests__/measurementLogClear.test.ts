import { describe, expect, it } from "vitest";
import { LOG_SESSION_NOTE, MeasurementLogCore, clearConfirmText, logSessionNote } from "../measurementLog";

describe("clearing the measurement log", () => {
  it("asks for the number of rows it will remove", () => {
    expect(clearConfirmText(12, 12)).toBe("Clear 12 rows?");
    expect(clearConfirmText(1, 1)).toBe("Clear 1 row?");
  });

  it("says every row goes when a filter shows only some of them", () => {
    expect(clearConfirmText(12, 3)).toBe("Clear all 12 rows, not only the 3 shown?");
  });

  it("tells the user the log lives only in this session", () => {
    expect(LOG_SESSION_NOTE).toBe("Kept for this session only: Save CSV to keep it.");
  });

  it("keeps the session-only warning with the number of rows at risk once the log holds rows", () => {
    expect(logSessionNote(3, 3)).toBe("Kept for this session only: Save CSV to keep these 3 rows.");
    expect(logSessionNote(1, 1)).toBe("Kept for this session only: Save CSV to keep this 1 row.");
  });

  it("says Save CSV writes only the shown rows while a filter hides some of them", () => {
    expect(logSessionNote(12, 3)).toBe("Kept for this session only: Save CSV writes the 3 shown rows of 12; clear the filters to keep them all.");
    expect(logSessionNote(12, 1)).toBe("Kept for this session only: Save CSV writes the 1 shown row of 12; clear the filters to keep them all.");
  });

  it("says the filters must be cleared before saving when they hide every row", () => {
    expect(logSessionNote(5, 0)).toBe("Kept for this session only: no row is shown; clear the filters to save the 5 rows with Save CSV.");
    expect(logSessionNote(1, 0)).toBe("Kept for this session only: no row is shown; clear the filters to save the 1 row with Save CSV.");
  });

  it("words the empty-log note and the row-count note with the same punctuation", () => {
    const prefix = "Kept for this session only: ";
    expect(LOG_SESSION_NOTE.startsWith(prefix)).toBe(true);
    for (const [total, shown] of [[3, 3], [12, 3], [5, 0]]) {
      expect(logSessionNote(total, shown).startsWith(prefix)).toBe(true);
    }
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
