import { describe, expect, it } from "vitest";
import { spccSyntheticNotice } from "../spccCatalog";

describe("spccSyntheticNotice", () => {
  it("does not blame the connection when the built-in catalogue was chosen", () => {
    const notice = spccSyntheticNotice("builtin");
    expect(notice).not.toMatch(/Gaia unavailable/);
    expect(notice).not.toMatch(/connection/);
    expect(notice).toMatch(/built-in catalogue/);
    expect(notice).toMatch(/not photometrically reliable/);
    expect(notice).toMatch(/Gaia DR3/);
  });

  it("keeps the Gaia fallback warning when Gaia was asked for and the synthetic catalogue was used", () => {
    const notice = spccSyntheticNotice("gaia");
    expect(notice).toMatch(/^Gaia unavailable/);
    expect(notice).toMatch(/Check your connection/);
    expect(notice).toMatch(/not photometrically reliable/);
  });
});
