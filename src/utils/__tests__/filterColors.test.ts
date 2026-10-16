import { describe, expect, it } from "vitest";
import metadataFileListSource from "../../components/file/MetadataFileList.tsx?raw";
import { FILTER_COLORS, getFilterColor } from "../filterColors";

const MUTED = "var(--ab-text-3)";

describe("FILTER_COLORS", () => {
  it("keeps the 27 JWST filters of the file list with their colours", () => {
    expect(Object.keys(FILTER_COLORS)).toHaveLength(27);
    expect(FILTER_COLORS.F070W).toBe("#dc2626");
    expect(FILTER_COLORS.F187N).toBe("#0d9488");
    expect(FILTER_COLORS.F322W2).toBe("#c026d3");
    expect(FILTER_COLORS.F444W).toBe("#991b1b");
    expect(FILTER_COLORS.F480M).toBe("#365314");
  });

  it("uses only six-digit hex colours", () => {
    for (const colour of Object.values(FILTER_COLORS)) expect(colour).toMatch(/^#[0-9a-f]{6}$/);
  });
});

describe("getFilterColor", () => {
  it("matches a filter name regardless of case and padding", () => {
    expect(getFilterColor("F444W")).toBe("#991b1b");
    expect(getFilterColor("f444w")).toBe("#991b1b");
    expect(getFilterColor("  F187N ")).toBe("#0d9488");
  });

  it("falls back to the muted text token for an unknown, empty or missing filter", () => {
    expect(getFilterColor("Ha")).toBe(MUTED);
    expect(getFilterColor("F502N")).toBe(MUTED);
    expect(getFilterColor("")).toBe(MUTED);
    expect(getFilterColor(undefined)).toBe(MUTED);
  });
});

describe("MetadataFileList", () => {
  it("imports the shared colour table instead of defining its own", () => {
    expect(metadataFileListSource).not.toMatch(/\bFILTER_COLORS\b/);
    expect(metadataFileListSource).not.toMatch(/function getFilterColor\b/);
    expect(metadataFileListSource).toMatch(/import \{ getFilterColor \} from "\.\.\/\.\.\/utils\/filterColors";/);
  });
});
