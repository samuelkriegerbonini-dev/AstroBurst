import { describe, expect, it } from "vitest";
import { binsFromMasters, laneForBin, PIPELINE_LANES } from "../pipelineToCompose";
import { DEFAULT_BINS, type FrequencyBin } from "../wizard";

const master = (label: string) => ({ label, fits_path: `C:/out/m42_pipeline12_20261030-101500-000_${label}.fits` });

function filesById(bins: FrequencyBin[]): Record<string, string[]> {
  return Object.fromEntries(bins.map((bin) => [bin.id, bin.files]));
}

function withFiles(bins: FrequencyBin[], files: Record<string, string[]>): FrequencyBin[] {
  return bins.map((bin) => ({ ...bin, files: files[bin.id] ?? [] }));
}

describe("binsFromMasters", () => {
  it("puts the R, G and B masters in the R, G and B bins and empties every other bin", () => {
    const before = withFiles(DEFAULT_BINS, { ha: ["C:/old/ha_1.fits", "C:/old/ha_2.fits"], r: ["C:/old/r_1.fits"] });
    const bins = binsFromMasters([master("R"), master("G"), master("B")], before);
    expect(filesById(bins)).toEqual({
      ha: [],
      oiii: [],
      sii: [],
      r: [master("R").fits_path],
      g: [master("G").fits_path],
      b: [master("B").fits_path],
      l: [],
    });
    expect(bins.map((bin) => [bin.id, bin.label, bin.color])).toEqual(DEFAULT_BINS.map((bin) => [bin.id, bin.label, bin.color]));
    expect(before.find((bin) => bin.id === "ha")?.files).toEqual(["C:/old/ha_1.fits", "C:/old/ha_2.fits"]);
  });

  it("puts the Ha, OIII and SII masters in the narrowband bins", () => {
    const bins = binsFromMasters([master("Ha"), master("OIII"), master("SII")], DEFAULT_BINS);
    expect(filesById(bins)).toMatchObject({
      ha: [master("Ha").fits_path],
      oiii: [master("OIII").fits_path],
      sii: [master("SII").fits_path],
      r: [],
      g: [],
      b: [],
    });
  });

  it("puts the L master in the luminance bin next to the colour masters", () => {
    const bins = binsFromMasters([master("L"), master("R"), master("G"), master("B")], DEFAULT_BINS);
    expect(filesById(bins)).toMatchObject({ l: [master("L").fits_path], r: [master("R").fits_path], b: [master("B").fits_path] });
  });

  it("puts a Mono master in the luminance bin when no L master comes with it", () => {
    expect(filesById(binsFromMasters([master("Mono")], DEFAULT_BINS)).l).toEqual([master("Mono").fits_path]);
  });

  it("gives a Mono master its own channel when an L master holds the luminance bin, whatever the order", () => {
    for (const masters of [[master("Mono"), master("L")], [master("L"), master("Mono")]]) {
      const bins = binsFromMasters(masters, DEFAULT_BINS);
      expect(filesById(bins).l).toEqual([master("L").fits_path]);
      expect(bins.slice(0, DEFAULT_BINS.length).map((bin) => bin.id)).toEqual(DEFAULT_BINS.map((bin) => bin.id));
      expect(bins[DEFAULT_BINS.length]).toEqual({ id: "mono", label: "Mono", shortLabel: "Mono", color: "#a1a1aa", files: [master("Mono").fits_path] });
    }
  });

  it("keeps custom bins, emptied, and adds back a default bin the wizard no longer has", () => {
    const nii: FrequencyBin = { id: "nii", label: "NII (658nm)", shortLabel: "NII", wavelength: 658, color: "hsl(120, 70%, 55%)", files: ["C:/old/nii.fits"] };
    const withoutSii = [...DEFAULT_BINS.filter((bin) => bin.id !== "sii"), nii];
    const bins = binsFromMasters([master("Ha"), master("SII")], withoutSii);
    expect(bins.map((bin) => bin.id)).toEqual([...withoutSii.map((bin) => bin.id), "sii"]);
    expect(bins.find((bin) => bin.id === "nii")).toEqual({ ...nii, files: [] });
    expect(bins.find((bin) => bin.id === "sii")).toEqual({ ...DEFAULT_BINS.find((bin) => bin.id === "sii"), files: [master("SII").fits_path] });
  });

  it("keeps the first master of a lane when two masters carry the same label", () => {
    const second = { label: "R", fits_path: "C:/out/second_R.fits" };
    expect(filesById(binsFromMasters([master("R"), second], DEFAULT_BINS)).r).toEqual([master("R").fits_path]);
  });
});

describe("laneForBin", () => {
  it("names the lane of every bin the pipeline stacks and none for the others", () => {
    expect(PIPELINE_LANES).toEqual(["L", "R", "G", "B", "Ha", "OIII", "SII", "Mono"]);
    expect(["l", "r", "g", "b", "ha", "oiii", "sii"].map(laneForBin)).toEqual(["L", "R", "G", "B", "Ha", "OIII", "SII"]);
    expect(laneForBin("nii")).toBeNull();
    expect(laneForBin(null)).toBeNull();
  });
});
