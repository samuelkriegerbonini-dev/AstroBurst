import { describe, expect, it } from "vitest";
import {
  NO_STAR_FWHM,
  STAR_FWHM_CARD_LABEL,
  STAR_FWHM_CARD_TITLE,
  STAR_FWHM_HEADER,
  STAR_FWHM_METHOD,
  medianStarFwhm,
  starFwhmText,
} from "../starListText";

const widths = (values: number[]) => values.map((fwhm) => ({ fwhm }));

describe("star list FWHM text", () => {
  it("takes the median of the listed widths", () => {
    expect(medianStarFwhm(widths([4.17, 4.09, 4.25]))).toBe(4.17);
    expect(medianStarFwhm(widths([4.0, 4.2]))).toBeCloseTo(4.1, 12);
    expect(medianStarFwhm(widths([3.3]))).toBe(3.3);
  });

  it("leaves out widths that are not a positive finite number", () => {
    expect(medianStarFwhm(widths([NaN, 0, -1, 3.3, Infinity]))).toBe(3.3);
    expect(medianStarFwhm([])).toBeNull();
    expect(medianStarFwhm(widths([NaN]))).toBeNull();
  });

  it("prints pixels, and arcsec when the image has a scale", () => {
    expect(starFwhmText(4.17, 0.0629)).toBe('4.17 px (0.262")');
    expect(starFwhmText(3.3, 0.396)).toBe('3.30 px (1.307")');
    for (const scale of [null, undefined, 0, -1, NaN]) expect(starFwhmText(4.17, scale)).toBe("4.17 px");
    expect(starFwhmText(null, 0.0629)).toBe("--");
  });

  it("names the column, the card and the method", () => {
    expect(STAR_FWHM_HEADER).toBe("FWHM (px)");
    expect(STAR_FWHM_CARD_LABEL).toBe("Median FWHM");
    expect(STAR_FWHM_METHOD).toBe(
      "FWHM from the second moment of each star's core (the pixels at or above half of its peak, at least its 9 brightest), corrected for the cut as for a Gaussian. A moment estimate, not a profile fit.",
    );
    expect(STAR_FWHM_CARD_TITLE).toBe(`Median over the listed stars. ${STAR_FWHM_METHOD}`);
    expect(STAR_FWHM_CARD_TITLE.endsWith(STAR_FWHM_METHOD)).toBe(true);
    expect(NO_STAR_FWHM).toBe("--");
  });
});
