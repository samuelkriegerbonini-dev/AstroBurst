import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

import type { MastError, MastFootprintInput, MastPeek, MastProductList, MastRow } from "../../../shared/types/mast";
import { NO_FOOTPRINT_TEXT, footprintInput } from "../../../utils/mastFootprints";
import { OverlayStoreCore } from "../../../utils/overlayStore";
import {
  PEEK_BUTTON_TEXT,
  PEEK_HINT,
  footprintBlockText,
  peekErrorText,
  peekKey,
  peekPolygons,
  peekRequestId,
  peekSummary,
  peekable,
  pixelType,
} from "../logic/peekView";
import { formatBytes } from "../logic/productView";
import { FOOTPRINT_NOTE, metadataSections } from "../logic/metadataView";
import { NO_WORKSPACE_TEXT } from "../logic/footprintView";
import { createPeeksState, createPeeksStore, datasetPeek, latestPeek, peekEntry, peeksReducer, peeksStore, type PeeksAction, type PeeksState } from "../state/peeksStore";
import { createPeeksController, type PeekDeps } from "../state/peeksController";
import { createFootprintsController, type FootprintDeps } from "../state/footprintsController";
import { createFootprintsStore, footprintsReducer, footprintsStore, shownInput } from "../state/footprintsStore";
import { archiveReducer, createInitialState, type ArchiveAction, type ArchiveState } from "../state/archiveReducer";
import type { ArchiveController } from "../state/archiveController";
import ProductsTab from "../ProductsTab";
import FootprintToggle from "../FootprintToggle";
import { MISSIONS, missionById, product, row } from "./fixtures";

const ROW_ID = "j9irw3fwq";
const FLC = "j9irw3fwq_flc.fits";
const DRC = "j9irw3fwq_drc.fits";
const CHIP_1: [number, number][] = [
  [150.0, 2.2],
  [150.06, 2.18],
  [150.08, 2.24],
  [150.02, 2.26],
];
const CHIP_2: [number, number][] = [
  [150.0, 2.25],
  [150.06, 2.23],
  [150.08, 2.29],
  [150.02, 2.31],
];

function peekOf(over: Partial<MastPeek> = {}): MastPeek {
  return {
    datasetId: ROW_ID,
    filename: FLC,
    uri: `mast:HST/product/${FLC}`,
    dims: [4096, 2048],
    bitpix: -32,
    bunit: "ELECTRONS",
    exptime: 507,
    dateObs: "2004-09-12T06:35:21",
    filter: "F606W",
    instrume: "ACS",
    footprints: [
      { hdu: 1, corners: CHIP_1, center: [150.04, 2.22], fovArcmin: [3.4, 1.7] },
      { hdu: 4, corners: CHIP_2, center: [150.04, 2.27], fovArcmin: [3.4, 1.7] },
    ],
    hdus: [
      { index: 1, extname: "SCI", extver: 1, bitpix: -32, dims: [4096, 2048], wcsError: null },
      { index: 4, extname: "SCI", extver: 2, bitpix: -32, dims: [4096, 2048], wcsError: null },
    ],
    bytesFetched: 524_288,
    ranges: 7,
    fileBytes: 168_000_000,
    complete: true,
    rangeIgnored: false,
    note: null,
    etag: '"ab"',
    elapsedMs: 900,
    fromCache: false,
    ...over,
  };
}

function hstRow(id = ROW_ID, over: Partial<MastRow> = {}): MastRow {
  return row(id, { instrument: "ACS", filter: "F606W", filters: ["F606W"], sRegion: null, program: "10398", values: { sci_data_set_name: id }, ...over });
}

function err(code: MastError["code"], message: string, context: MastError["context"] = {}): MastError {
  return { code, message, context };
}

function run(state: PeeksState, ...actions: PeeksAction[]): PeeksState {
  return actions.reduce(peeksReducer, state);
}

function hstProducts(datasetId: string): MastProductList {
  const products = [
    product(FLC, "image", 168_000_000, { datasetId, mastUri: `mast:HST/product/${FLC}` }),
    product(DRC, "image", 214_000_000, { datasetId, mastUri: `mast:HST/product/${DRC}`, badges: ["hap_aligned"] }),
    product("j9irw3fwq_flc.fits.gz", "image", 90_000_000, { datasetId, mastUri: "mast:HST/product/j9irw3fwq_flc.fits.gz" }),
    product("j9irw3fwq_spt.fits", "auxiliary", 30_000, { datasetId, mastUri: "mast:HST/product/j9irw3fwq_spt.fits" }),
  ];
  return {
    mission: "hst",
    datasetIds: [datasetId],
    products,
    groups: [
      { cls: "image", count: 3, bytes: 472_000_000, defaultVisible: true },
      { cls: "auxiliary", count: 1, bytes: 30_000, defaultVisible: false },
    ],
    totalCount: products.length,
    totalBytes: products.reduce((sum, p) => sum + p.sizeBytes, 0),
    elapsedMs: 1200,
    fromCache: false,
    cachedUntil: null,
  };
}

function productsState(datasetId: string): ArchiveState {
  const key = `hst:${datasetId}`;
  const actions: ArchiveAction[] = [
    { type: "missionsLoaded", missions: MISSIONS },
    { type: "productsStarted", key, requestId: `g1:products:${datasetId}`, at: 0 },
    { type: "productsLoaded", key, requestId: `g1:products:${datasetId}`, list: hstProducts(datasetId) },
  ];
  return actions.reduce(archiveReducer, createInitialState());
}

const ctl = new Proxy({}, { get: (target: Record<string | symbol, unknown>, prop) => (target[prop] ??= vi.fn()) }) as unknown as ArchiveController;

function tags(markup: string, testid: string): string[] {
  return [...markup.matchAll(new RegExp(`<[a-z]+[^>]*data-testid="${testid}"[^>]*>`, "g"))].map((m) => m[0]);
}

describe("peek view", () => {
  it("offers a peek only for FITS images of missions whose products have a mast: URI", () => {
    const flc = product(FLC, "image", 1, { mastUri: `mast:HST/product/${FLC}` });
    expect(peekable(flc, "hst")).toBe(true);
    expect(peekable(product("a_i2d.fits", "image", 1), "jwst")).toBe(true);
    expect(peekable(product("a_i2d.fits.fz", "image", 1), "jwst")).toBe(true);
    expect(peekable(product("j9irw3fwq_flc.fits.gz", "image", 1), "hst")).toBe(false);
    expect(peekable(product("a_x1d.fits", "spectrum", 1), "jwst")).toBe(false);
    expect(peekable(product("a_cat.ecsv", "catalog", 1), "jwst")).toBe(false);
    expect(peekable(product("r_cal.fits", "image", 1), "roman")).toBe(false);
    expect(peekable(product("swp_mxlo.fits", "image", 1, { mastUri: "mast:IUE/url/swp.fits" }), "iue")).toBe(false);
    expect(peekable(product("hlsp_x_i2d.fits", "image", 1, { mastUri: "mast:HLSP/ceers/nircam1/hlsp_x_i2d.fits" }), "iue")).toBe(true);
  });

  it("keys a peek by mission, dataset and file and names a valid request id", () => {
    expect(peekKey("hst", ROW_ID, FLC)).toBe(`hst:${ROW_ID}:${FLC}`);
    expect(peekRequestId(ROW_ID, FLC)).toBe(`peek:${ROW_ID}:${FLC}`);
    expect(peekRequestId(ROW_ID, FLC)).toMatch(/^[A-Za-z0-9:._-]{1,512}$/);
  });

  it("names the pixel type of BITPIX", () => {
    expect([-32, -64, 8, 16, 32, 64, 12, null].map(pixelType)).toEqual(["float32", "float64", "uint8", "int16", "int32", "int64", null, null]);
  });

  it("summarises size, type, instrument, exposure, footprints and what the peek read", () => {
    expect(peekSummary(peekOf())).toBe(`4096 x 2048 float32 · 2 science extensions · ACS F606W · 507 s · 2 footprints · ${formatBytes(524_288)} in 7 ranges`);
    expect(peekSummary(peekOf({ fromCache: true }))).toBe("4096 x 2048 float32 · 2 science extensions · ACS F606W · 507 s · 2 footprints · from the cache");
    const jwst = peekOf({
      dims: [2048, 2048],
      exptime: 161.052,
      instrume: "NIRCAM",
      filter: "F200W",
      footprints: [peekOf().footprints[0]],
      hdus: [peekOf().hdus[0]],
      bytesFetched: 131_072,
      ranges: 1,
    });
    expect(peekSummary(jwst)).toBe(`2048 x 2048 float32 · NIRCAM F200W · 161.1 s · 1 footprint · ${formatBytes(131_072)} in 1 range`);
    const partial = peekOf({ complete: false, footprints: [peekOf().footprints[0]], hdus: [peekOf().hdus[0]], dims: [], bitpix: null, instrume: null, filter: null, exptime: null });
    expect(peekSummary(partial)).toBe(`1 footprint · partial · ${formatBytes(524_288)} in 7 ranges`);
    expect(peekSummary(peekOf({ footprints: [] }))).toContain("no footprint");
  });

  it("turns the footprint corners into sky polygons", () => {
    expect(peekPolygons(peekOf())).toEqual([CHIP_1, CHIP_2]);
  });

  it("says what went wrong with a peek in one sentence", () => {
    expect(peekErrorText(err("mast_peek_incomplete", "no science header within the first 524288 bytes (7 ranges)", { bytes: 524_288 }))).toBe(
      "No science header within the first 524288 bytes (7 ranges). Download the file to read its headers.",
    );
    expect(peekErrorText(err("mast_timeout", "no answer within 60 s", { timeoutSec: 60 }))).toBe("MAST did not send the header within 60 s. Try again in a moment.");
    expect(peekErrorText(err("mast_decode", "the file header could not be read: the bytes at offset 0 are not a FITS primary header (first keyword '<!DOCTYP')", { operation: "api/v0.1/Download/file" }))).toBe(
      "The file header could not be read: the bytes at offset 0 are not a FITS primary header (first keyword '<!DOCTYP').",
    );
    expect(peekErrorText(err("mast_server", "HTTP 503", { status: 503 }))).toBe("MAST had a problem answering (HTTP 503). This is on MAST's side; try again in a moment.");
    expect(peekErrorText(err("mast_unauthorized", "authorization required", { status: 401 }))).toBe("This file is in exclusive access at MAST.");
    expect(peekErrorText(err("mast_not_deployed", "not deployed (HTTP 404)", { status: 404 }))).toBe("MAST has no file at this URI (HTTP 404).");
    expect(peekErrorText(err("mast_network", "connection refused", { host: "mast.stsci.edu" }))).toBe("Could not reach MAST (mast.stsci.edu): connection refused.");
    expect(peekErrorText(err("mast_bad_input", "filename: only .fits, .fit, .fts and .fits.fz files have a header that can be read in part"))).toBe(
      "filename: only .fits, .fit, .fts and .fits.fz files have a header that can be read in part.",
    );
  });

  it("points rows without an s_region to the header peek when their mission has one", () => {
    expect(footprintBlockText(NO_FOOTPRINT_TEXT, "hst")).toBe(`${NO_FOOTPRINT_TEXT}. ${PEEK_HINT}`);
    expect(footprintBlockText(NO_FOOTPRINT_TEXT, "iue")).toBe(NO_FOOTPRINT_TEXT);
    expect(footprintBlockText("Moving target: the archive footprint is not sidereal", "hst")).toBe("Moving target: the archive footprint is not sidereal");
    expect(footprintBlockText(null, "hst")).toBeNull();
  });
});

describe("peeks store", () => {
  const flcKey = peekKey("hst", ROW_ID, FLC);
  const drcKey = peekKey("hst", ROW_ID, DRC);

  it("follows one attempt per file from loading to ready or failed and ignores stale answers", () => {
    const started = run(createPeeksState(), { type: "started", key: flcKey, mission: "hst", datasetId: ROW_ID, filename: FLC, attempt: 1 });
    expect(peekEntry(started, "hst", ROW_ID, FLC)?.stage).toBe("loading");
    expect(run(started, { type: "loaded", key: flcKey, attempt: 0, peek: peekOf() })).toBe(started);
    const ready = run(started, { type: "loaded", key: flcKey, attempt: 1, peek: peekOf() });
    expect(peekEntry(ready, "hst", ROW_ID, FLC)).toMatchObject({ stage: "ready", peek: peekOf(), error: null });
    const failed = run(ready, { type: "started", key: drcKey, mission: "hst", datasetId: ROW_ID, filename: DRC, attempt: 2 }, { type: "failed", key: drcKey, attempt: 2, error: err("mast_server", "HTTP 503", { status: 503 }) });
    expect(peekEntry(failed, "hst", ROW_ID, DRC)).toMatchObject({ stage: "failed", error: { code: "mast_server" } });
    expect(peekEntry(failed, "hst", ROW_ID, FLC)?.stage).toBe("ready");
  });

  it("forgets a cancelled first peek and keeps the earlier result of a cancelled repeat", () => {
    const cancelled = run(
      createPeeksState(),
      { type: "started", key: flcKey, mission: "hst", datasetId: ROW_ID, filename: FLC, attempt: 1 },
      { type: "dropped", key: flcKey, attempt: 1 },
    );
    expect(peekEntry(cancelled, "hst", ROW_ID, FLC)).toBeNull();
    const repeated = run(
      createPeeksState(),
      { type: "started", key: flcKey, mission: "hst", datasetId: ROW_ID, filename: FLC, attempt: 1 },
      { type: "loaded", key: flcKey, attempt: 1, peek: peekOf() },
      { type: "started", key: flcKey, mission: "hst", datasetId: ROW_ID, filename: FLC, attempt: 2 },
    );
    expect(peekEntry(repeated, "hst", ROW_ID, FLC)).toMatchObject({ stage: "loading", peek: peekOf() });
    expect(peekEntry(run(repeated, { type: "dropped", key: flcKey, attempt: 2 }), "hst", ROW_ID, FLC)).toMatchObject({ stage: "ready", peek: peekOf() });
  });

  it("gives the footprints of the dataset's preferred complete peek that has any", () => {
    const drc = peekOf({ filename: DRC, footprints: [peekOf().footprints[0]], hdus: [peekOf().hdus[0]] });
    const bare = peekOf({ filename: "j9irw3fwq_flt.fits", footprints: [] });
    const s = run(
      createPeeksState(),
      { type: "started", key: flcKey, mission: "hst", datasetId: ROW_ID, filename: FLC, attempt: 1 },
      { type: "started", key: drcKey, mission: "hst", datasetId: ROW_ID, filename: DRC, attempt: 2 },
      { type: "loaded", key: drcKey, attempt: 2, peek: drc },
      { type: "loaded", key: flcKey, attempt: 1, peek: peekOf() },
      { type: "started", key: peekKey("hst", ROW_ID, bare.filename), mission: "hst", datasetId: ROW_ID, filename: bare.filename, attempt: 3 },
      { type: "loaded", key: peekKey("hst", ROW_ID, bare.filename), attempt: 3, peek: bare },
      { type: "started", key: peekKey("hst", "other", FLC), mission: "hst", datasetId: "other", filename: FLC, attempt: 4 },
      { type: "loaded", key: peekKey("hst", "other", FLC), attempt: 4, peek: peekOf({ datasetId: "other" }) },
    );
    expect(datasetPeek(s, "hst", ROW_ID)?.filename).toBe(DRC);
    expect(latestPeek(s, "hst", ROW_ID)?.filename).toBe(bare.filename);
    const partialKey = peekKey("hst", ROW_ID, "j9irw3fwq_flt.fits");
    const partial = run(
      s,
      { type: "started", key: partialKey, mission: "hst", datasetId: ROW_ID, filename: "j9irw3fwq_flt.fits", attempt: 5 },
      { type: "loaded", key: partialKey, attempt: 5, peek: peekOf({ filename: "j9irw3fwq_flt.fits", complete: false, footprints: [peekOf().footprints[0]] }) },
    );
    expect(latestPeek(partial, "hst", ROW_ID)?.complete).toBe(false);
    expect(datasetPeek(partial, "hst", ROW_ID)?.filename).toBe(DRC);
    expect(datasetPeek(s, "jwst", ROW_ID)).toBeNull();
    expect(datasetPeek(createPeeksState(), "hst", ROW_ID)).toBeNull();
  });

  it("prefers a drizzled _drc or _drz peek over a _flc or _flt whatever came last, then the latest", () => {
    const drz = "j9irw3fwq_drz.fits";
    const drzKey = peekKey("hst", ROW_ID, drz);
    const fltKey = peekKey("hst", ROW_ID, "j9irw3fwq_flt.fits");
    const flcLast = run(
      createPeeksState(),
      { type: "started", key: drcKey, mission: "hst", datasetId: ROW_ID, filename: DRC, attempt: 1 },
      { type: "loaded", key: drcKey, attempt: 1, peek: peekOf({ filename: DRC, footprints: [peekOf().footprints[0]] }) },
      { type: "started", key: flcKey, mission: "hst", datasetId: ROW_ID, filename: FLC, attempt: 2 },
      { type: "loaded", key: flcKey, attempt: 2, peek: peekOf() },
    );
    expect(datasetPeek(flcLast, "hst", ROW_ID)?.filename).toBe(DRC);
    expect(latestPeek(flcLast, "hst", ROW_ID)?.filename).toBe(FLC);
    const partialDrz = run(
      createPeeksState(),
      { type: "started", key: flcKey, mission: "hst", datasetId: ROW_ID, filename: FLC, attempt: 1 },
      { type: "loaded", key: flcKey, attempt: 1, peek: peekOf() },
      { type: "started", key: drzKey, mission: "hst", datasetId: ROW_ID, filename: drz, attempt: 2 },
      { type: "loaded", key: drzKey, attempt: 2, peek: peekOf({ filename: drz, complete: false, footprints: [peekOf().footprints[0]] }) },
    );
    expect(datasetPeek(partialDrz, "hst", ROW_ID)?.filename).toBe(FLC);
    const fltLast = run(
      partialDrz,
      { type: "started", key: fltKey, mission: "hst", datasetId: ROW_ID, filename: "j9irw3fwq_flt.fits", attempt: 3 },
      { type: "loaded", key: fltKey, attempt: 3, peek: peekOf({ filename: "j9irw3fwq_flt.fits" }) },
    );
    expect(datasetPeek(fltLast, "hst", ROW_ID)?.filename).toBe("j9irw3fwq_flt.fits");
    const drzDone = run(
      fltLast,
      { type: "started", key: drzKey, mission: "hst", datasetId: ROW_ID, filename: drz, attempt: 4 },
      { type: "loaded", key: drzKey, attempt: 4, peek: peekOf({ filename: drz, footprints: [peekOf().footprints[0]] }) },
    );
    expect(datasetPeek(drzDone, "hst", ROW_ID)?.filename).toBe(drz);
  });

  it("keeps the dataset's footprint while its peek is repeated and after the repeat failed", () => {
    const repeating = run(
      createPeeksState(),
      { type: "started", key: flcKey, mission: "hst", datasetId: ROW_ID, filename: FLC, attempt: 1 },
      { type: "loaded", key: flcKey, attempt: 1, peek: peekOf() },
      { type: "started", key: flcKey, mission: "hst", datasetId: ROW_ID, filename: FLC, attempt: 2 },
    );
    expect(peekEntry(repeating, "hst", ROW_ID, FLC)?.stage).toBe("loading");
    expect(datasetPeek(repeating, "hst", ROW_ID)?.filename).toBe(FLC);
    const failed = run(repeating, { type: "failed", key: flcKey, attempt: 2, error: err("mast_network", "connection reset", { host: "mast.stsci.edu" }) });
    expect(peekEntry(failed, "hst", ROW_ID, FLC)?.stage).toBe("failed");
    expect(datasetPeek(failed, "hst", ROW_ID)?.filename).toBe(FLC);
  });
});

describe("peeks controller", () => {
  it("asks for the product's header under its request id and stores the result", async () => {
    const store = createPeeksStore();
    const peek = vi.fn<PeekDeps["peek"]>(async () => peekOf());
    const ctl = createPeeksController(store, { peek });
    const flc = hstProducts("j9irw3fwq").products[0];
    const pending = ctl.fetch("hst", ROW_ID, flc);
    expect(peekEntry(store.getState(), "hst", ROW_ID, FLC)?.stage).toBe("loading");
    await pending;
    expect(peek).toHaveBeenCalledWith(`peek:${ROW_ID}:${FLC}`, "hst", "j9irw3fwq", FLC, null);
    expect(peekEntry(store.getState(), "hst", ROW_ID, FLC)).toMatchObject({ stage: "ready", peek: peekOf() });
    const hlsp = product("hlsp_ceers_i2d.fits", "image", 1, { datasetId: "ceers", mastUri: "mast:HLSP/ceers/nircam1/hlsp_ceers_i2d.fits" });
    await ctl.fetch("jwst", "ceers", hlsp);
    expect(peek).toHaveBeenLastCalledWith("peek:ceers:hlsp_ceers_i2d.fits", "jwst", "ceers", "hlsp_ceers_i2d.fits", "mast:HLSP/ceers/nircam1/hlsp_ceers_i2d.fits");
  });

  it("keeps a failure on the product and forgets a cancelled peek", async () => {
    const store = createPeeksStore();
    const answers = [err("mast_server", "HTTP 503", { status: 503 }), err("mast_cancelled", "cancelled")];
    const ctl = createPeeksController(store, {
      peek: async () => {
        throw answers.shift();
      },
    });
    const flc = hstProducts("j9irw3fwq").products[0];
    await ctl.fetch("hst", ROW_ID, flc);
    expect(peekEntry(store.getState(), "hst", ROW_ID, FLC)).toMatchObject({ stage: "failed", error: { code: "mast_server", context: { status: 503 } } });
    await ctl.fetch("hst", ROW_ID, flc);
    expect(peekEntry(store.getState(), "hst", ROW_ID, FLC)).toBeNull();
  });
});

describe("footprints from a header peek", () => {
  const field = { path: "C:\\data\\field.fits", name: "field.fits", projectionPath: "C:\\data\\field.fits" };

  it("feeds the peek polygons to rows without an s_region and leaves the s_region rows alone", () => {
    const polygons = peekPolygons(peekOf());
    expect(footprintInput(hstRow(), polygons)).toEqual({ datasetId: ROW_ID, sRegion: null, polygons, targtype: null });
    expect(footprintInput(hstRow(), [])).toMatchObject({ polygons: null });
    const jwst = row("jw02739-o001_t001_nircam");
    expect(footprintInput(jwst, polygons)).toMatchObject({ sRegion: jwst.sRegion, polygons: null });
  });

  it("projects and saves the peek polygons through the footprint controller", async () => {
    const project = vi.fn<FootprintDeps["project"]>(async (path, items) => ({
      path,
      width: 4096,
      height: 4096,
      footprints: items.map((item) => ({ datasetId: item.datasetId, polygons: [[[0, 0], [1, 0], [1, 1]]], onImage: true, vertices: 8, cone: null, note: null })),
    }));
    const regText = vi.fn<FootprintDeps["regText"]>(async () => ({ regText: "icrs\n", polygons: 2, notes: [] }));
    const ctl = createFootprintsController(createFootprintsStore(), {
      project,
      regText,
      exportImage: async () => "image\n",
      chooseFile: async (name) => `C:\\out\\${name}`,
      writeText: async () => {},
      overlay: new OverlayStoreCore(),
    });
    const polygons = peekPolygons(peekOf());
    expect(await ctl.save(hstRow(), polygons)).toBe(`C:\\out\\${ROW_ID}_footprint.reg`);
    expect(regText).toHaveBeenCalledWith([{ datasetId: ROW_ID, sRegion: null, polygons, targtype: null }]);
    await ctl.setWorkspace(field, "present");
    await ctl.toggle(hstRow(), polygons);
    const sent: MastFootprintInput[] = project.mock.calls[0][1];
    expect(sent).toEqual([{ datasetId: ROW_ID, sRegion: null, polygons, targtype: null }]);
  });

  it("enables the toggle of an HST row once a peek gave it a footprint", () => {
    const id = "j9irw4b1q";
    const before = renderToStaticMarkup(createElement(FootprintToggle, { row: hstRow(id), mission: "hst" }));
    expect(before).toContain(PEEK_HINT);
    const key = peekKey("hst", id, `${id}_flc.fits`);
    peeksStore.dispatch({ type: "started", key, mission: "hst", datasetId: id, filename: `${id}_flc.fits`, attempt: 901 });
    peeksStore.dispatch({ type: "loaded", key, attempt: 901, peek: peekOf({ datasetId: id, filename: `${id}_flc.fits` }) });
    const after = renderToStaticMarkup(createElement(FootprintToggle, { row: hstRow(id), mission: "hst" }));
    expect(after).not.toContain(NO_FOOTPRINT_TEXT);
    expect(after).toContain(NO_WORKSPACE_TEXT);
  });

  function loadPeek(id: string, peek: MastPeek, attempt: number) {
    const key = peekKey("hst", id, peek.filename);
    peeksStore.dispatch({ type: "started", key, mission: "hst", datasetId: id, filename: peek.filename, attempt });
    peeksStore.dispatch({ type: "loaded", key, attempt, peek });
  }

  function toggleMarkup(id: string): { container: string; toggle: string; markup: string } {
    const markup = renderToStaticMarkup(createElement(FootprintToggle, { row: hstRow(id), mission: "hst" }));
    return { container: tags(markup, "mast-footprint")[0], toggle: tags(markup, "mast-footprint-toggle")[0], markup };
  }

  function clickToggle(id: string) {
    const best = datasetPeek(peeksStore.getState(), "hst", id);
    const input = shownInput(footprintsStore.getState(), footprintInput(hstRow(id), best ? peekPolygons(best) : null));
    return footprintsReducer(footprintsStore.getState(), { type: "toggle", input: footprintInput(hstRow(id), input.polygons) });
  }

  it("keeps the toggle on the drawn footprint when another product of the row is peeked", () => {
    const id = "j9irw6abq";
    const flc = peekOf({ datasetId: id, filename: `${id}_flc.fits` });
    const drc = peekOf({ datasetId: id, filename: `${id}_drc.fits`, footprints: [{ hdu: 1, corners: CHIP_2, center: [150.04, 2.27], fovArcmin: [3.4, 3.4] }] });
    footprintsStore.dispatch({ type: "workspace", file: field, wcs: "present" });
    loadPeek(id, flc, 921);
    footprintsStore.dispatch({ type: "toggle", input: footprintInput(hstRow(id), peekPolygons(flc)) });
    expect(toggleMarkup(id).container).toContain('data-shown="true"');
    loadPeek(id, drc, 922);
    const after = toggleMarkup(id);
    expect(after.container).toContain('data-shown="true"');
    expect(after.toggle).toContain('aria-pressed="true"');
    const clicked = clickToggle(id);
    expect(clicked.items.filter((item) => item.datasetId === id)).toEqual([]);
    footprintsStore.dispatch({ type: "clear" });
    footprintsStore.dispatch({ type: "workspace", file: null, wcs: "absent" });
  });

  it("keeps the toggle enabled and on while the shown product is peeked again and after the repeat failed", () => {
    const id = "j9irw7acq";
    const flc = peekOf({ datasetId: id, filename: `${id}_flc.fits` });
    const key = peekKey("hst", id, flc.filename);
    footprintsStore.dispatch({ type: "workspace", file: field, wcs: "present" });
    loadPeek(id, flc, 931);
    footprintsStore.dispatch({ type: "toggle", input: footprintInput(hstRow(id), peekPolygons(flc)) });
    peeksStore.dispatch({ type: "started", key, mission: "hst", datasetId: id, filename: flc.filename, attempt: 932 });
    const loading = toggleMarkup(id);
    expect(loading.container).toContain('data-shown="true"');
    expect(loading.toggle).not.toContain('disabled=""');
    expect(loading.markup).not.toContain(NO_FOOTPRINT_TEXT);
    peeksStore.dispatch({ type: "failed", key, attempt: 932, error: err("mast_network", "connection reset", { host: "mast.stsci.edu" }) });
    const failed = toggleMarkup(id);
    expect(failed.container).toContain('data-shown="true"');
    expect(failed.toggle).not.toContain('disabled=""');
    expect(failed.markup).not.toContain(NO_FOOTPRINT_TEXT);
    expect(clickToggle(id).items.filter((item) => item.datasetId === id)).toEqual([]);
    footprintsStore.dispatch({ type: "clear" });
    footprintsStore.dispatch({ type: "workspace", file: null, wcs: "absent" });
  });
});

describe("Products tab peeks", () => {
  it("offers Fetch footprint on FITS images and shows what each peek found", () => {
    const id = "j9irw5kaq";
    const markup = () => renderToStaticMarkup(createElement(ProductsTab, { state: productsState(id), ctl, row: hstRow(id), mission: missionById("hst") }));
    const fresh = markup();
    const buttons = tags(fresh, "mast-peek-fetch");
    expect(buttons).toHaveLength(2);
    expect(buttons[0]).toContain(`aria-label="${PEEK_BUTTON_TEXT}: ${FLC}"`);
    expect(tags(fresh, "mast-peek")).toEqual([]);

    const flcKey = peekKey("hst", id, FLC);
    const drcKey = peekKey("hst", id, DRC);
    peeksStore.dispatch({ type: "started", key: flcKey, mission: "hst", datasetId: id, filename: FLC, attempt: 911 });
    peeksStore.dispatch({ type: "loaded", key: flcKey, attempt: 911, peek: peekOf({ datasetId: id, note: "HDU 7 has no usable WCS: missing CRPIX1" }) });
    peeksStore.dispatch({ type: "started", key: drcKey, mission: "hst", datasetId: id, filename: DRC, attempt: 912 });
    peeksStore.dispatch({ type: "failed", key: drcKey, attempt: 912, error: err("mast_server", "HTTP 503", { status: 503 }) });
    const shown = markup();
    const [ready, failed] = tags(shown, "mast-peek");
    expect(ready).toContain('data-state="ready"');
    expect(ready).toContain(`data-file="${FLC}"`);
    expect(ready).toContain('data-bytes="524288"');
    expect(ready).toContain('data-ranges="7"');
    expect(ready).toContain('data-footprints="2"');
    expect(ready).toContain('data-complete="true"');
    expect(ready).toContain('data-cache="false"');
    expect(shown).toContain(peekSummary(peekOf()));
    expect(shown).toContain("HDU 7 has no usable WCS: missing CRPIX1");
    expect(failed).toContain('data-state="failed"');
    expect(failed).toContain('role="alert"');
    expect(shown).toContain("MAST had a problem answering (HTTP 503)");

    const loadingKey = peekKey("hst", id, FLC);
    peeksStore.dispatch({ type: "started", key: loadingKey, mission: "hst", datasetId: id, filename: FLC, attempt: 913 });
    const busy = markup();
    expect(tags(busy, "mast-peek-fetch")[0]).toContain("disabled");
    expect(tags(busy, "mast-peek")[0]).toContain('data-state="loading"');
  });
});

describe("Metadata from a header peek", () => {
  it("lists the peek summary and the peek footprint of a row without an s_region", () => {
    const peek = { filename: FLC, summary: peekSummary(peekOf()), polygons: 2 };
    const sections = metadataSections({ row: hstRow(), resolved: null, searchPosition: null, footprintNote: null, peek });
    const pointing = sections.find((s) => s.key === "pointing")?.rows ?? [];
    expect(pointing.find((r) => r.label === "Footprint")).toEqual({ label: "Footprint", value: `2 polygons from the ${FLC} header`, copy: null, note: FOOTPRINT_NOTE });
    const observation = sections.find((s) => s.key === "observation")?.rows ?? [];
    expect(observation.find((r) => r.label === "Header peek")).toEqual({ label: "Header peek", value: peek.summary, copy: null, note: FLC });
    const withRegion = metadataSections({ row: row("jw02739-o001_t001_nircam"), resolved: null, searchPosition: null, footprintNote: "1 polygon", peek: { ...peek, polygons: 0 } });
    const regionRows = withRegion.find((s) => s.key === "pointing")?.rows ?? [];
    expect(regionRows.map((r) => r.label)).toContain("s_region");
    expect(regionRows.map((r) => r.label)).not.toContain("Footprint");
  });
});
