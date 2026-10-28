import { describe, expect, it } from "vitest";
import { TauriCommandError } from "../../shared/types/errors";
import type { PlateSolveResult } from "../../services/astrometry";
import {
  NO_API_KEY_NOTICE,
  ORIENTATION_TITLE,
  SIP_RESIDUAL_TITLE,
  deadlineFailure,
  deadlineText,
  failureHeading,
  failureLines,
  fallbackOffer,
  formatDuration,
  localFailure,
  orientationText,
  resultBlockFields,
  solveTierLabel,
  solvingText,
} from "../plateSolveResult";

const LOCAL_RESULT: PlateSolveResult = {
  center_ra: 274.70012,
  center_dec: -13.84988,
  orientation: 91.2,
  pixel_scale_arcsec: 2.39,
  field_of_view_w_arcmin: 238.6,
  field_of_view_h_arcmin: 159.1,
  wcs_cards: [["CTYPE1", "'RA---TAN-SIP'"]],
  solver: "local",
  catalog: "tier0-tycho2",
  matched: 293,
  predicted: 410,
  rms_px: 0.0684,
  sip_order: 3,
  sip_max_err_px: 0.0041,
  hint_source: "OBJCTRA/OBJCTDEC",
  elapsed_ms: 778,
  parity: "normal",
};

const NOVA_RESULT: PlateSolveResult = {
  center_ra: 190.03,
  center_dec: -11.63,
  orientation: 12.5,
  pixel_scale_arcsec: 0.0629,
  field_of_view_w_arcmin: 2.24,
  field_of_view_h_arcmin: 2.31,
  wcs_cards: [],
};

const NO_SOLUTION = "No solution: the best candidate matched 14 stars with an RMS of 6.27 px";

function noSolution(context: Record<string, unknown>): TauriCommandError {
  return new TauriCommandError("plate_solve_cmd", NO_SOLUTION, "local_no_solution", context);
}

function field(result: PlateSolveResult, key: string): string | undefined {
  return resultBlockFields(result).find((f) => f.key === key)?.value;
}

describe("solveTierLabel", () => {
  it("names the tier and the catalogue the solver labels the solution with", () => {
    expect(solveTierLabel("tier0-tycho2")).toBe("Tier 0: Tycho-2 (bundled)");
    expect(solveTierLabel("tier1-2mass")).toBe("Tier 1: 2MASS");
    expect(solveTierLabel("tier1-gaia-siril")).toBe("Tier 1: Gaia DR3 (Siril extract)");
    expect(solveTierLabel("tier2-gaia-vizier")).toBe("Tier 2: Gaia DR3 online (VizieR)");
  });

  it("reads the tier from the result's tier field when the catalogue is a dataset id or a name", () => {
    expect(solveTierLabel("tycho2-l6-v1", 0)).toBe("Tier 0: Tycho-2 (bundled)");
    expect(solveTierLabel("Tycho-2", 0)).toBe("Tier 0: Tycho-2 (bundled)");
    expect(solveTierLabel("2mass-l8-v1", 1)).toBe("Tier 1: 2MASS");
    expect(solveTierLabel("siril-gaia-astro-v1", 1)).toBe("Tier 1: Gaia DR3 (Siril extract)");
    expect(solveTierLabel("gaia-dr3", 2)).toBe("Tier 2: Gaia DR3 online (VizieR)");
    expect(solveTierLabel(null, 2)).toBe("Tier 2");
    expect(field({ ...LOCAL_RESULT, catalog: "tycho2-l6-v1", tier: 0 }, "catalog")).toBe("Tier 0: Tycho-2 (bundled)");
    expect(field({ ...LOCAL_RESULT, catalog: null, tier: 1 }, "catalog")).toBe("Tier 1");
  });

  it("keeps an unknown label readable and gives nothing for an empty one", () => {
    expect(solveTierLabel("tier3-future")).toBe("Tier 3: future");
    expect(solveTierLabel("ucac4")).toBe("ucac4");
    expect(solveTierLabel("")).toBeNull();
    expect(solveTierLabel(null)).toBeNull();
    expect(solveTierLabel(undefined)).toBeNull();
  });
});

describe("orientationText (UX-17)", () => {
  it("prints the measured angle east of north and marks a mirrored solve", () => {
    expect(orientationText(91.2, "normal")).toBe("91.20° E of N");
    expect(orientationText(-3.456, "flipped")).toBe("-3.46° E of N (mirrored)");
    expect(orientationText(12.5)).toBe("12.50° E of N");
  });

  it("prints -- instead of a fake 0 when the solver gave no orientation", () => {
    expect(orientationText(null, "normal")).toBe("--");
    expect(orientationText(undefined)).toBe("--");
    expect(orientationText(Number.NaN)).toBe("--");
  });

  it("states the convention in the card title", () => {
    expect(ORIENTATION_TITLE).toMatch(/east of north/);
    expect(ORIENTATION_TITLE).toMatch(/FITS \+y/);
  });

  it("tells the reader that the WCS readout's CROTA2 rotation has the opposite sign", () => {
    expect(ORIENTATION_TITLE).toMatch(/CROTA2/);
    expect(ORIENTATION_TITLE).toMatch(/opposite sign/);
  });
});

describe("resultBlockFields", () => {
  it("result_block_fields_list_tier_matched_rms_and_sip", () => {
    expect(resultBlockFields(LOCAL_RESULT).map((f) => [f.key, f.label, f.value])).toEqual([
      ["solver", "Solved by", "Offline solver on this computer"],
      ["catalog", "Catalogue", "Tier 0: Tycho-2 (bundled)"],
      ["matched", "Matched", "293 of 410 predicted"],
      ["rms", "RMS", "0.068 px"],
      ["sip", "Distortion", "SIP order 3 (SIPMXERR 0.0041 px)"],
      ["orientation", "Orientation", "91.20° E of N"],
      ["hint", "Hint", "OBJCTRA/OBJCTDEC"],
      ["elapsed", "Time", "0.78 s"],
    ]);
    expect(resultBlockFields(LOCAL_RESULT).find((f) => f.key === "orientation")?.title).toBe(ORIENTATION_TITLE);
  });

  it("names the SIP residual as the SIPMXERR card and says it is not the error on the sky", () => {
    const sip = resultBlockFields(LOCAL_RESULT).find((f) => f.key === "sip");
    expect(sip?.value).not.toMatch(/\bmax\b/);
    expect(sip?.title).toBe(SIP_RESIDUAL_TITLE);
    expect(SIP_RESIDUAL_TITLE).toMatch(/SIPMXERR/);
    expect(SIP_RESIDUAL_TITLE).toMatch(/not the error on the sky/);
    expect(resultBlockFields({ ...LOCAL_RESULT, sip_order: null }).find((f) => f.key === "sip")?.title).toBeUndefined();
  });

  it("says a local solve without distortion terms is TAN only", () => {
    expect(field({ ...LOCAL_RESULT, sip_order: null, sip_max_err_px: null }, "sip")).toBe("TAN only (no SIP)");
    expect(field({ ...LOCAL_RESULT, sip_order: 0 }, "sip")).toBe("TAN only (no SIP)");
    expect(field({ ...LOCAL_RESULT, sip_max_err_px: null }, "sip")).toBe("SIP order 3");
  });

  it("keeps an astrometry.net result to the fields nova gives and still shows the orientation card", () => {
    expect(resultBlockFields(NOVA_RESULT).map((f) => [f.key, f.value])).toEqual([["orientation", "12.50° E of N"]]);
    expect(resultBlockFields({ ...NOVA_RESULT, solver: "astrometry.net", orientation: null }).map((f) => [f.key, f.value])).toEqual([
      ["solver", "astrometry.net"],
      ["orientation", "--"],
    ]);
  });

  it("shows matched stars without a predicted count when the solver gives none", () => {
    expect(field({ ...LOCAL_RESULT, predicted: null }, "matched")).toBe("293 stars");
    expect(field({ ...LOCAL_RESULT, matched: null }, "matched")).toBeUndefined();
  });
});

describe("localFailure", () => {
  it("reads the local no-solution error and its best candidate in either key spelling", () => {
    const failure = localFailure(
      noSolution({
        best: { matched: 14, predicted: 120, rms_px: 6.27, centre: [274.1, -13.2] },
        expectedTier0Stars: 20.4,
        centresTried: 397,
        detections: 286,
        canFallback: true,
      }),
    );
    expect(failure).toEqual({
      kind: "no_solution",
      message: NO_SOLUTION,
      best: { matched: 14, predicted: 120, rmsPx: 6.27 },
      expectedTier0Stars: 20.4,
      centresTried: 397,
      detections: 286,
      canFallback: true,
      apiKeyStored: null,
      minMatches: null,
      maxRmsPx: null,
      tier0MinExpectedStars: null,
    });
    expect(
      localFailure(noSolution({ best: { matched: 3, predicted: 50, rmsPx: 2 }, expected_tier0_stars: 75, centres_tried: 1, can_fallback: false })),
    ).toMatchObject({ best: { matched: 3, predicted: 50, rmsPx: 2 }, expectedTier0Stars: 75, centresTried: 1, canFallback: false });
  });

  it("gives nothing for an error that is not a local solve failure", () => {
    expect(localFailure(new Error("network down"))).toBeNull();
    expect(localFailure("timeout")).toBeNull();
    expect(localFailure(new TauriCommandError("plate_solve_cmd", "astrometry.net: HTTP 500", "nova_failed"))).toBeNull();
    expect(localFailure(new TauriCommandError("plate_solve_cmd", "Plate solve cancelled", "cancelled"))).toBeNull();
    expect(localFailure(null)).toBeNull();
  });

  it("offers the fallback when the error does not say otherwise", () => {
    expect(localFailure(noSolution({}))).toEqual({
      kind: "no_solution",
      message: NO_SOLUTION,
      best: null,
      expectedTier0Stars: null,
      centresTried: null,
      detections: null,
      canFallback: true,
      apiKeyStored: null,
      minMatches: null,
      maxRmsPx: null,
      tier0MinExpectedStars: null,
    });
    expect(localFailure(new TauriCommandError("plate_solve_cmd", NO_SOLUTION, "local_no_solution"))?.canFallback).toBe(true);
  });

  it.each([
    ["local_no_solution", "no_solution"],
    ["local_missing_hint", "missing_hint"],
    ["local_no_catalogue", "no_catalogue"],
  ] as const)("reads the backend's %s error as a local failure that offers astrometry.net", (code, kind) => {
    const failure = localFailure(
      new TauriCommandError("plate_solve_cmd", "x", code, { canFallback: true, apiKeyStored: false, missing: ["centre"] }),
    );
    expect(failure).toMatchObject({ kind, canFallback: true, apiKeyStored: false });
    expect(fallbackOffer(failure, "auto", true).offered).toBe(true);
  });

  it("reads whether the backend found a stored API key", () => {
    expect(localFailure(noSolution({ apiKeyStored: true }))?.apiKeyStored).toBe(true);
    expect(localFailure(noSolution({ api_key_stored: false }))?.apiKeyStored).toBe(false);
  });
});

const MISSING_HINT =
  "The offline solver needs an approximate centre and none was found in the header or the panel; type it in the hint fields, or try astrometry.net, which can solve without hints";
const NO_CATALOGUE =
  "No offline star catalogue is available (tier0-tycho2: the bundled Tycho-2 catalogue was not found in this installation); install one in Settings > Catalogs";

describe("failureHeading and failureLines", () => {
  it("states the reason, the best candidate, the search and the tier-0 star count", () => {
    const failure = localFailure(
      noSolution({ best: { matched: 14, predicted: 120, rms_px: 6.274 }, expectedTier0Stars: 20.4, centresTried: 397, detections: 286 }),
    )!;
    expect(failureLines(failure)).toEqual([
      "Searched 397 centres with 286 detected stars.",
      "Best candidate: 14 of 120 predicted stars matched, RMS 6.27 px; a solution needs 8 matches and an RMS of 1 px or less.",
      `${NO_SOLUTION}.`,
      "About 20 Tycho-2 stars are expected in this field, fewer than tier 0 needs; add a deeper catalogue in Settings > Catalogs, such as a Siril Gaia DR3 extract.",
    ]);
  });

  it("puts the search summary first and names an unreachable server by its host, not its full request URL", () => {
    const failure = localFailure(
      new TauriCommandError(
        "plate_solve_cmd",
        "No offline solution. tier2-gaia-vizier: catalogue read failed: VizieR request failed: error sending request for url (http://127.0.0.1:9/?-source=I%2F355%2Fgaiadr3&-c=117.182536+%2B51.011914&-c.r=1.3756&-c.u=deg&-out.max=9447&-sort=Gmag&Gmag=%3C17.00); tier0-tycho2: no solution within 3.00 deg of the hint after 397 centres: no centre produced 8 consistent star matches",
        "local_no_solution",
        { centresTried: 416, detections: 150, best: null, expectedTier0Stars: 180 },
      ),
    )!;
    const lines = failureLines(failure);
    expect(lines[0]).toBe("Searched 416 centres with 150 detected stars.");
    expect(lines.join("\n")).not.toMatch(/https?:\/\//);
    expect(lines[1]).toBe(
      "Tier 2: Gaia DR3 online (VizieR): catalogue read failed: VizieR request failed: error sending request for 127.0.0.1:9; Tier 0: Tycho-2 (bundled): no solution within 3.00 deg of the hint after 397 centres: no centre produced 8 consistent star matches.",
    );
  });

  it("quotes the acceptance thresholds the backend sends instead of its own copies", () => {
    const failure = localFailure(
      noSolution({
        best: { matched: 5, predicted: 40, rms_px: 1.2 },
        expectedTier0Stars: 45,
        minMatches: 6,
        maxRmsPx: 1.5,
        tier0MinExpectedStars: 40,
      }),
    )!;
    expect(failureLines(failure)).toEqual([
      "Best candidate: 5 of 40 predicted stars matched, RMS 1.20 px; a solution needs 6 matches and an RMS of 1.5 px or less.",
      `${NO_SOLUTION}.`,
    ]);
  });

  it("leaves out what the error does not carry and does not suggest a deeper catalogue for a rich field", () => {
    expect(failureLines(localFailure(noSolution({ expectedTier0Stars: 180 }))!)).toEqual([`${NO_SOLUTION}.`]);
  });

  it("does not repeat the heading and names the tiers tried in words instead of solver labels", () => {
    const failure = localFailure(
      new TauriCommandError(
        "plate_solve_cmd",
        "No offline solution. tier0-tycho2: no solution within 3 degrees; tier2-gaia-vizier: 12 usable stars detected, at least 15 needed",
        "local_no_solution",
        {},
      ),
    )!;
    expect(failureHeading(failure)).toBe("No offline solution");
    expect(failureLines(failure)).toEqual([
      "Tier 0: Tycho-2 (bundled): no solution within 3 degrees; Tier 2: Gaia DR3 online (VizieR): 12 usable stars detected, at least 15 needed.",
    ]);
  });

  it("words a missing centre and a missing catalogue by their own heading, without the tier-0 star hint", () => {
    const missing = localFailure(new TauriCommandError("plate_solve_cmd", MISSING_HINT, "local_missing_hint", { missing: ["centre"] }))!;
    expect(failureHeading(missing)).toBe("Not enough hints to solve offline");
    expect(failureLines(missing)).toEqual([`${MISSING_HINT}.`]);
    const none = localFailure(new TauriCommandError("plate_solve_cmd", NO_CATALOGUE, "local_no_catalogue", { expectedTier0Stars: 3 }))!;
    expect(failureHeading(none)).toBe("No offline catalogue");
    expect(failureLines(none)).toEqual([
      "No offline star catalogue is available (Tier 0: Tycho-2 (bundled): the bundled Tycho-2 catalogue was not found in this installation); install one in Settings > Catalogs.",
    ]);
  });
});

describe("deadlineFailure", () => {
  it("turns the panel's offline deadline into a local failure that offers astrometry.net", () => {
    const failure = deadlineFailure("The offline solve gave up after 2:00 (Settings > Timeout).");
    expect(failure).toMatchObject({ kind: "deadline", canFallback: true, best: null });
    expect(failureHeading(failure)).toBe("The offline solve ran out of time");
    expect(failureLines(failure)).toEqual(["The offline solve gave up after 2:00 (Settings > Timeout)."]);
    expect(fallbackOffer(failure, "auto", true).offered).toBe(true);
  });
});

describe("fallbackOffer", () => {
  const failure = localFailure(noSolution({ canFallback: true }));

  it("fallback_state_is_offered_only_after_a_local_no_solution_error", () => {
    expect(fallbackOffer(failure, "auto", true)).toEqual({
      offered: true,
      note: "Uploads this image to nova.astrometry.net with your API key; the solve runs there.",
    });
    expect(fallbackOffer(failure, "local", true).offered).toBe(true);
    expect(fallbackOffer(null, "auto", true).offered).toBe(false);
    expect(fallbackOffer(localFailure(new Error("disk full")), "auto", true).offered).toBe(false);
    expect(fallbackOffer(failure, "astrometry.net", true).offered).toBe(false);
    expect(fallbackOffer(failure, null, true).offered).toBe(false);
    expect(fallbackOffer(localFailure(noSolution({ canFallback: false })), "auto", true).offered).toBe(false);
  });

  it("says the upload needs a key without one, and leaves the button to check the key at the click", () => {
    expect(fallbackOffer(failure, "auto", false)).toEqual({
      offered: true,
      note: "Uploads this image to nova.astrometry.net and needs an astrometry.net API key: save it in Settings, then click Try astrometry.net.",
    });
    expect(NO_API_KEY_NOTICE).toBe("No astrometry.net API key is saved, so nothing was uploaded. Save it in Settings, then click Try astrometry.net again.");
  });

  it("names the configured astrometry.net host", () => {
    expect(fallbackOffer(failure, "auto", true, "https://astro.example.org/api").note).toBe(
      "Uploads this image to astro.example.org with your API key; the solve runs there.",
    );
    expect(fallbackOffer(failure, "auto", true, "not a url").note).toContain("nova.astrometry.net");
  });

  it("offers nothing and says nothing when it is not offered", () => {
    expect(fallbackOffer(null, "auto", false)).toEqual({ offered: false, note: "" });
  });
});

describe("solver-aware running texts", () => {
  it("says the offline solve stays on this computer", () => {
    expect(solvingText("local", "2:00")).toBe("Solving on this computer with the offline catalogues; nothing is uploaded. Gives up after 2:00.");
    expect(deadlineText("local", "2:00")).toBe("The offline solve gave up after 2:00 (Settings > Timeout).");
  });

  it("says the default solve keeps the image here and may fetch a Gaia DR3 star list from VizieR", () => {
    expect(solvingText("auto", "2:00")).toBe(
      "Solving on this computer; the image is not uploaded. When the offline catalogues are not deep enough, the Gaia DR3 star list of the field is fetched from VizieR. Gives up after 2:00.",
    );
    expect(deadlineText("auto", "2:00")).toBe("The offline solve gave up after 2:00 (Settings > Timeout).");
  });

  it("keeps the astrometry.net wording for the fallback", () => {
    expect(solvingText("astrometry.net", "2:00")).toBe("Queued on astrometry.net; the request gives up after 2:00.");
    expect(deadlineText("astrometry.net", "2:00")).toBe(
      "Plate solve gave up after 2:00 (Settings > Timeout). astrometry.net may still finish the job; try again later.",
    );
  });
});

describe("formatDuration", () => {
  it("writes whole seconds as m:ss and never goes below zero", () => {
    const cases: [number, string][] = [
      [0, "0:00"],
      [41_900, "0:41"],
      [150_000, "2:30"],
      [600_000, "10:00"],
      [-5, "0:00"],
    ];
    for (const [ms, text] of cases) expect(formatDuration(ms), String(ms)).toBe(text);
  });
});
