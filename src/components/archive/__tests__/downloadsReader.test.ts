import { describe, expect, it, vi } from "vitest";

const { readable } = vi.hoisted(() => ({ readable: vi.fn<(name: string) => boolean>() }));

vi.mock("../../../utils/validation", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../../utils/validation")>();
  readable.mockImplementation(actual.isValidFitsFile);
  return { ...actual, isValidFitsFile: readable };
});

import { openableDownload } from "../logic/downloadView";

const SPECTRUM = { opensAs: "spectrum" as const, filename: "hlsp_ullyses_hst_cos_av-sco_g130m-g160m_dr7_spec.fits.gz" };

describe("Open follows the files the Files panel accepts", () => {
  it("opens a .fits.gz spectrum once the reader lists it and keeps catalogues download-only", () => {
    readable.mockImplementation((name) => name.endsWith(".fits.gz"));
    expect(openableDownload(SPECTRUM)).toBe(true);
    expect(openableDownload({ opensAs: "catalog", filename: "x_cat.fits.gz" })).toBe(false);
    readable.mockImplementation(() => false);
    expect(openableDownload(SPECTRUM)).toBe(false);
  });
});
