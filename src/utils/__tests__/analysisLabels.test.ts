import { describe, it, expect } from "vitest";
import {
  FFT_FREQUENCY_UNIT,
  MEAN_LABEL,
  SIGMA_MAD_LABEL,
  SIGMA_MAD_TITLE,
  fftFrequencyReadout,
  fftGridTitle,
  fftSizeLabel,
  skyRangeLabel,
  starCountLabel,
} from "../analysisLabels";

describe("histogram footer labels", () => {
  it("names the robust sigma and explains it", () => {
    expect(SIGMA_MAD_LABEL).toBe("σ(MAD)");
    expect(SIGMA_MAD_TITLE).toBe("1.4826 × MAD (robust)");
  });

  it("uses the Greek letters, never a literal escape", () => {
    expect(MEAN_LABEL).toBe("μ");
    expect(SIGMA_MAD_LABEL.codePointAt(0)).toBe(0x03c3);
    for (const label of [MEAN_LABEL, SIGMA_MAD_LABEL, SIGMA_MAD_TITLE]) expect(label).not.toContain("\\");
  });

  it("prints the sky window with an en dash", () => {
    expect(skyRangeLabel({ lo: 120, hi: 230.25 })).toBe("sky 120.0–230.3");
    expect(skyRangeLabel({ lo: 0.00001234, hi: 0.5 })).toBe("sky 0.00001234–0.5000");
  });
});

describe("robust sigma label sources", () => {
  const components = import.meta.glob<string>("../../**/*.tsx", { query: "?raw", import: "default", eager: true });
  const localCopy = /1\.4826|σ\(MAD\)["=]|&sigma;\(MAD\)/;

  it("scans the component sources", () => {
    expect(Object.keys(components).length).toBeGreaterThan(100);
  });

  it("has no component with its own copy of the σ(MAD) label or title", () => {
    const offenders = Object.entries(components)
      .filter(([, text]) => localCopy.test(text))
      .map(([name]) => name);
    expect(offenders).toEqual([]);
  });
});

describe("starCountLabel", () => {
  it("shows the plain count when nothing was cut", () => {
    expect(starCountLabel(143, 143)).toBe("143");
    expect(starCountLabel(143, null)).toBe("143");
    expect(starCountLabel(143, undefined)).toBe("143");
  });

  it("names the cap when more stars were detected than kept", () => {
    expect(starCountLabel(200, 1532)).toBe("200 of 1532 (brightest)");
  });

  it("ignores a total that is not above the shown count", () => {
    expect(starCountLabel(200, 150)).toBe("200");
    expect(starCountLabel(200, Number.NaN)).toBe("200");
  });
});

describe("fftSizeLabel", () => {
  it("names the measured image, not the padded grid, as the source of a downsampled spectrum", () => {
    expect(fftSizeLabel({ width: 1024, height: 512, imageWidth: 6000, imageHeight: 3000, downsampled: true })).toBe(
      "1024×512 (from 6000×3000)",
    );
  });

  it("prints no source size when the spectrum was not downsampled", () => {
    expect(fftSizeLabel({ width: 512, height: 1024, imageWidth: 300, imageHeight: 1000, downsampled: false })).toBe(
      "512×1024",
    );
  });

  it("uses the multiplication sign, never a literal escape", () => {
    const label = fftSizeLabel({ width: 256, height: 256, imageWidth: 2000, imageHeight: 2000, downsampled: true });
    expect(label).not.toContain("\\u");
    expect(label).toContain("×");
  });
});

describe("fftGridTitle", () => {
  it("states the zero-padded grid the transform ran on", () => {
    expect(fftGridTitle({ gridWidth: 8192, gridHeight: 4096 })).toBe(
      "FFT grid 8192×4096: the image zero-padded to the next power of two on each axis",
    );
  });
});

describe("fftFrequencyReadout", () => {
  it("labels the spatial frequency in cycles per pixel", () => {
    expect(FFT_FREQUENCY_UNIT).toBe("cyc/px");
    expect(fftFrequencyReadout("0.125", "-0.250")).toBe("freq (0.125, -0.250) cyc/px");
  });
});
