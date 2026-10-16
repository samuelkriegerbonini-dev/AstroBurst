import { describe, expect, it } from "vitest";
import {
  EXCLUSIVE_PREVIEWS_TEXT,
  PRODUCTS_SLOW_NOTE,
  QUICKLOOK_NOTE,
  datasetPositionText,
  detailHeader,
  entriesInGroup,
  entryFilename,
  entryTitle,
  groupsOf,
  largeNeeded,
  largeTitle,
  listFailureText,
  listingText,
  parseStripKey,
  previewAlt,
  previewFailureText,
  previewInfoLines,
  previewKey,
  previewStatusText,
  previewsAllowed,
  productsFailureText,
  productsListingText,
  progressFraction,
  progressText,
  railImageLines,
  railObservationLines,
  sourceWcs,
  stepEntry,
  stripKey,
  stripPosition,
  stripThumbKeys,
  stripThumbUrl,
  tabLabels,
} from "../logic/detailView";
import { pixelScaleText } from "../logic/quickWcs";
import { hstSet, jwstL3Set, missionById, productList, row } from "./fixtures";
import type { MastImageEntry, MastPreviewRef } from "../../../shared/types/mast";

const L3 = jwstL3Set();
const F187N = L3.entries[1];

function paneRef(over: Partial<MastPreviewRef> = {}): MastPreviewRef {
  return {
    requestId: "g1:preview:jw02739-o001_t001_nircam:f187n.jpg:pane",
    path: "C:\\cache\\mast\\img\\jwst\\jw02739-o001_t001_nircam\\f187n.2048.jpg",
    width: 2048,
    height: 1226,
    bytes: 900_000,
    filename: "f187n.2048.jpg",
    wcs: { centreRaDeg: 274.7, centreDecDeg: -13.85, pixelScaleArcsec: (0.031 * 14341) / 2048, fieldArcmin: [7.41, 4.46], orientation: "unreliable", ctype: "RA---TAN" },
    imageCount: 6,
    elapsedMs: 2100,
    fromCache: false,
    size: "pane",
    channels: 1,
    source: { width: 14341, height: 8583, bytes: 5_800_000 },
    ...over,
  };
}

function calEntry(): MastImageEntry {
  return { ...F187N, key: "cal-along", label: "nrcalong", group: "cal", suffix: "cal", detector: "nrcalong", filter: "F444W" };
}

describe("identifiers", () => {
  it("keys previews by mission, dataset, file and size, and filmstrip thumbnails by dataset and file", () => {
    expect(previewKey("jwst", "jw02739-o001_t001_nircam", "a.jpg", "pane")).toBe("jwst:jw02739-o001_t001_nircam:a.jpg:pane");
    expect(stripKey("jw02739-o001_t001_nircam", "a_thumb.jpg")).toBe("jw02739-o001_t001_nircam:a_thumb.jpg");
    expect(parseStripKey("jw02739-o001_t001_nircam:a_thumb.jpg")).toEqual({ datasetId: "jw02739-o001_t001_nircam", filename: "a_thumb.jpg" });
    expect(parseStripKey("jw02739-o001_t001_nircam")).toBeNull();
  });

  it("names an image by its preview file, or its product file when there is no preview", () => {
    expect(entryFilename(F187N)).toBe(F187N.previewFilename);
    expect(entryFilename({ ...F187N, previewFilename: null })).toBe(F187N.productFilename);
  });

  it("titles an image by its label and suffix without repeating the suffix", () => {
    expect(entryTitle(F187N)).toBe("F187N i2d");
    expect(entryTitle(calEntry())).toBe("nrcalong cal");
    expect(entryTitle(hstSet().entries[0])).toBe("F502N combined");
    expect(entryTitle({ ...hstSet().entries[0], label: "ick909030_drc", suffix: "drc" })).toBe("ick909030_drc");
  });
});

describe("detail header", () => {
  it("lists mission, instrument, exposure type, level and access, then programme, observation and target", () => {
    const h = detailHeader(row("jw02739-o001_t001_nircam"), "JWST");
    expect(h.badges).toEqual(["JWST", "NIRCAM", "NRC_IMAGE", "L3", "PUBLIC"]);
    expect(h.line).toBe("Programme 2739 · Obs 1 · M 16");
    expect(h.exclusive).toBeNull();
  });

  it("adds the single filter of a dataset without a level and marks exclusive access with its end date", () => {
    const hst = detailHeader(row("ICK909030", { levelClass: "unknown", level: null, instrument: "WFC3", expType: null, filter: "F502N", filters: ["F502N"], program: "15238", observation: null, target: "M16" }), "HST");
    expect(hst.badges).toEqual(["HST", "WFC3", "F502N", "PUBLIC"]);
    expect(hst.line).toBe("Programme 15238 · M16");
    const excl = detailHeader(row("jw01234-o001_t001_miri", { instrument: "MIRI", access: "PROPRIETARY", releaseDate: "2027-03-02T00:00:00" }), "JWST");
    expect(excl.badges).toEqual(["JWST", "MIRI", "NRC_IMAGE", "L3"]);
    expect(excl.exclusive).toBe("EXCLUSIVE until 2027-03-02");
  });
});

describe("tabs and access", () => {
  it("counts previews and products once known and marks missing previews", () => {
    expect(tabLabels({ set: null, products: null, previewsEnabled: true })).toEqual({ previews: "Previews", metadata: "Metadata", products: "Products" });
    expect(tabLabels({ set: L3, products: productList(), previewsEnabled: true })).toEqual({ previews: "Previews 6", metadata: "Metadata", products: "Products 10" });
    expect(tabLabels({ set: null, products: null, previewsEnabled: false }).previews).toBe("Previews (none)");
  });

  it("allows previews for public data of missions with images, and for exclusive data only with a token mission", () => {
    const pub = row("a");
    const excl = row("b", { access: "PROPRIETARY" });
    expect(previewsAllowed(pub, missionById("jwst"))).toBe(true);
    expect(previewsAllowed(excl, missionById("jwst"))).toBe(false);
    expect(previewsAllowed(excl, missionById("roman"))).toBe(true);
    expect(previewsAllowed(pub, missionById("iue"))).toBe(false);
    expect(EXCLUSIVE_PREVIEWS_TEXT).toBe("MAST serves no preview of exclusive-access data to anonymous users.");
  });
});

describe("groups and filmstrip", () => {
  it("lists only groups with images and the entries of one group", () => {
    const hst = hstSet();
    expect(groupsOf({ ...hst, groups: [...hst.groups, { key: "ima", label: "Intermediate", count: 0 }] }).map((g) => g.key)).toEqual(["combined", "calibrated", "raw", "hap"]);
    expect(entriesInGroup(hst, "combined").map((e) => e.key)).toEqual(["ick909030_drc", "ick909030_drz"]);
  });

  it("steps through the group without wrapping and gives the position", () => {
    expect(stepEntry(L3, F187N.key, 1)).toBe(L3.entries[2].key);
    expect(stepEntry(L3, F187N.key, -1)).toBe(L3.entries[0].key);
    expect(stepEntry(L3, L3.entries[0].key, -1)).toBeNull();
    expect(stepEntry(L3, L3.entries[5].key, 1)).toBeNull();
    expect(stripPosition(L3, F187N)).toEqual({ index: 2, count: 6 });
    const hst = hstSet();
    expect(stepEntry(hst, "ick909030_drz", 1)).toBeNull();
  });

  it("asks for filmstrip thumbnails the card thumbnail does not already cover", () => {
    const card = { url: "http://asset.localhost/card.jpg", width: 128, height: 77, imageCount: 6, filename: F187N.thumbnailFilename ?? "" };
    const keys = stripThumbKeys("jw02739-o001_t001_nircam", L3.entries, card);
    expect(keys).toHaveLength(5);
    expect(keys).not.toContain(stripKey("jw02739-o001_t001_nircam", F187N.thumbnailFilename ?? ""));
    expect(stripThumbKeys("jw02739-o001_t001_nircam", [{ ...F187N, thumbnailFilename: null }], undefined)).toEqual([]);
    const refs = { [stripKey("jw02739-o001_t001_nircam", L3.entries[0].thumbnailFilename ?? "")]: { ...card, url: "http://asset.localhost/strip0.jpg" } };
    expect(stripThumbUrl("jw02739-o001_t001_nircam", L3.entries[0], refs, card)).toBe("http://asset.localhost/strip0.jpg");
    expect(stripThumbUrl("jw02739-o001_t001_nircam", F187N, refs, card)).toBe("http://asset.localhost/card.jpg");
    expect(stripThumbUrl("jw02739-o001_t001_nircam", L3.entries[3], refs, card)).toBeNull();
  });
});

describe("preview text", () => {
  it("describes a resized preview: source, shown size, original bytes, source pixel scale and field", () => {
    expect(previewInfoLines(F187N, paneRef())).toEqual([
      "F187N i2d · 14341 x 8583 source",
      "shown 2048 x 1226 · 5.8 MB",
      "0.031\"/px · field 7.4' x 4.5'",
      QUICKLOOK_NOTE,
    ]);
    expect(QUICKLOOK_NOTE).toBe("8-bit quicklook made by MAST, not science data");
  });

  it("describes a preview shown at its own size and names a CD-matrix orientation", () => {
    const ref = paneRef({
      width: 2048,
      height: 2048,
      source: { width: 2048, height: 2048, bytes: 1_100_000 },
      wcs: { centreRaDeg: 274.7, centreDecDeg: -13.85, pixelScaleArcsec: 0.0628, fieldArcmin: [2.14, 2.14], orientation: "cd_matrix", ctype: "RA---TAN" },
    });
    expect(previewInfoLines(calEntry(), ref)).toEqual(["nrcalong cal · 2048 x 2048 · 1.1 MB", "0.063\"/px · field 2.1' x 2.1'", "orientation from the CD matrix", QUICKLOOK_NOTE]);
    expect(previewInfoLines(calEntry(), { ...ref, wcs: null })).toEqual(["nrcalong cal · 2048 x 2048 · 1.1 MB", QUICKLOOK_NOTE]);
  });

  it("converts the copy's pixel scale back to source pixels", () => {
    const wcs = sourceWcs(paneRef());
    expect(wcs).not.toBeNull();
    expect(wcs?.pixelScaleArcsec).toBeCloseTo(0.031, 10);
    expect(pixelScaleText(wcs!)).toBe('0.031"/px');
    expect(sourceWcs(paneRef({ wcs: null }))).toBeNull();
  });

  it("asks for a 4096 px copy only when the pane copy was reduced", () => {
    expect(largeNeeded(paneRef())).toBe(true);
    expect(largeNeeded(paneRef({ width: 1870, height: 2048, source: { width: 1870, height: 2048, bytes: 3_100_000 } }))).toBe(false);
  });

  it("writes alt text with the source size and the shown long edge", () => {
    expect(previewAlt(F187N, paneRef())).toBe("F187N i2d preview, 14341 by 8583 pixels, shown at 2048");
  });

  it("reports download progress against the total when MAST sends one", () => {
    expect(progressText({ received: 2_200_000, total: 5_800_000, startedAt: 1000 }, 3100)).toBe("2.2 of 5.8 MB · 2.1 s");
    expect(progressText({ received: 2_200_000, total: null, startedAt: 1000 }, 3100)).toBe("2.2 MB · 2.1 s");
    expect(progressText({ received: 0, total: null, startedAt: 1000 }, 1000)).toBe("0 B · 0.0 s");
    expect(progressFraction({ received: 2_900_000, total: 5_800_000 })).toBe(0.5);
    expect(progressFraction({ received: 10, total: null })).toBeNull();
  });

  it("says what is happening per stage and while the list loads", () => {
    expect(previewStatusText("download", F187N, "pane")).toBe("Downloading F187N preview");
    expect(previewStatusText("decode", F187N, "pane")).toBe("Preparing a 2048 px copy");
    expect(previewStatusText("decode", F187N, "large")).toBe("Preparing a 4096 px copy");
    expect(listingText("jw02739-o001_t001_nircam", 800)).toBe("Listing images for jw02739-o001_t001_nircam … 0.8 s");
  });

  it("words the product listing and failures of lists and listings", () => {
    expect(productsListingText("jw02739-o001_t001_nircam", 12_000)).toBe("Listing products for jw02739-o001_t001_nircam … 12 s");
    expect(PRODUCTS_SLOW_NOTE).toBe("MAST takes 20 to 30 s for listings it has not seen recently.");
    expect(listFailureText({ code: "mast_network", message: "connection refused", context: {} })).toBe("Could not reach MAST (mast.stsci.edu): connection refused.");
    expect(productsFailureText({ code: "mast_server", message: "x", context: { status: 504 } })).toBe("MAST had a problem answering (HTTP 504). This is on MAST's side; try again in a moment.");
  });

  it("explains a failed preview with MAST's reason or the size", () => {
    expect(previewFailureText({ code: "mast_no_image", message: "404", context: { status: 404, detail: "Image not found" } }, true)).toBe("No preview for this image (MAST: Image not found).");
    expect(previewFailureText({ code: "mast_too_large", message: "cap", context: { bytes: 71_000_000 } }, true)).toBe(
      "This image is larger than AstroBurst previews (71 MB). It will be available through downloads.",
    );
    expect(previewFailureText({ code: "mast_cancelled", message: "", context: {} }, true)).toBe("Preview cancelled.");
  });
});

describe("Large view text", () => {
  it("titles the view with target, mission, instrument and dataset, and gives the position in the results", () => {
    expect(largeTitle(row("jw02739-o001_t001_nircam"), "JWST")).toBe("M 16 · JWST NIRCam · jw02739-o001_t001_nircam");
    expect(largeTitle(row("x", { target: null, instrument: null }), "HST")).toBe("HST · x");
    expect(datasetPositionText(0, 2)).toBe("dataset 1 of 2");
  });

  it("fills the info rail with the image and the observation", () => {
    const large = paneRef({ width: 4096, height: 2451, size: "large", wcs: { ...paneRef().wcs!, pixelScaleArcsec: (0.031 * 14341) / 4096 } });
    expect(railImageLines(F187N, large)).toEqual(["F187N i2d · level 3", "source 14341 x 8583", "shown 4096 x 2451", "5.8 MB from MAST", "0.031\"/px · 7.4' x 4.5'", "orientation not given"]);
    expect(railObservationLines(row("jw02739-o001_t001_nircam"))).toEqual(["Programme 2739 · Obs 1", "2022-08-14 · 5797.9 s", QUICKLOOK_NOTE]);
  });
});
