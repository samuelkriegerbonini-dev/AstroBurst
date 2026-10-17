import { describe, expect, it, vi } from "vitest";
import { createElement, type ComponentType } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import DetailPane from "../DetailPane";
import LargeView from "../LargeView";
import ProductsTab from "../ProductsTab";
import { archiveReducer, createInitialState, type ArchiveAction, type ArchiveState } from "../state/archiveReducer";
import type { ArchiveController } from "../state/archiveController";
import { EXCLUSIVE_PREVIEWS_TEXT, previewKey } from "../logic/detailView";
import { L3_FILTERS, MISSIONS, jwstL3Set, page, product, productList, row } from "./fixtures";
import type { MastError, MastPreviewRef, MastQuery, MastResolved, MastRow } from "../../../shared/types/mast";

const M16: MastResolved = { query: "M16", canonicalName: "M 16", resolver: "SIMBAD", raDeg: 274.688, decDeg: -13.792, objectType: null, elapsedMs: 870, fromCache: false };
const QUERY: MastQuery = {
  mission: "jwst",
  where: { kind: "position", raDeg: 274.688, decDeg: -13.792, radiusArcmin: 3, label: "M 16" },
  program: null,
  datasetId: null,
  filters: { level: "combined" },
  preset: null,
  sort: [],
  offset: 0,
  limit: 200,
};
const ID = "jw02739-o001_t001_nircam";
const KEY = `jwst:${ID}`;
const L3 = jwstL3Set(ID);
const F187N = L3.entries[1];
const PANE = previewKey("jwst", ID, F187N.previewFilename ?? "", "pane");
const LARGE = previewKey("jwst", ID, F187N.previewFilename ?? "", "large");
const ROWS = [row(ID), row("jw02739-o002_t001_miri", { instrument: "MIRI", filters: ["F770W", "F1130W"] })];

function stubController(): ArchiveController {
  const fns: Record<string | symbol, unknown> = {};
  return new Proxy(fns, { get: (target, prop) => (target[prop] ??= vi.fn()) }) as unknown as ArchiveController;
}

const ctl = stubController();

function run(state: ArchiveState, ...actions: ArchiveAction[]): ArchiveState {
  return actions.reduce(archiveReducer, state);
}

function base(): ArchiveState {
  return run(createInitialState(), { type: "missionsLoaded", missions: MISSIONS }, { type: "tokenStatus", status: { present: false, authorized: null } });
}

function searched(rows: MastRow[] = ROWS, state = base()): ArchiveState {
  return run(
    { ...state, form: { ...state.form, text: "M16" } },
    { type: "searchStarted", generation: 1, text: "M16", kind: "name", name: "M16", query: null, at: 0 },
    { type: "nameResolved", generation: 1, resolved: M16, query: QUERY, at: 900 },
    { type: "pageLoaded", generation: 1, page: page(rows) },
  );
}

function withList(s = searched()): ArchiveState {
  return run(s, { type: "imageListLoaded", key: KEY, set: L3 });
}

function ref(size: "pane" | "large"): MastPreviewRef {
  const width = size === "pane" ? 2048 : 4096;
  const height = size === "pane" ? 1226 : 2451;
  return {
    requestId: `g1:preview:${ID}:${F187N.previewFilename}:${size}`,
    path: `C:\\cache\\mast\\img\\jwst\\${ID}\\f187n.${width}.jpg`,
    width,
    height,
    bytes: 900_000,
    filename: `f187n.${width}.jpg`,
    wcs: { centreRaDeg: 274.7, centreDecDeg: -13.85, pixelScaleArcsec: (0.031 * 14341) / width, fieldArcmin: [7.41, 4.46], orientation: "unreliable", ctype: "RA---TAN" },
    imageCount: 6,
    elapsedMs: 2100,
    fromCache: false,
    size,
    channels: 1,
    source: { width: 14341, height: 8583, bytes: 5_800_000 },
  };
}

function ready(s: ArchiveState, key: string, size: "pane" | "large"): ArchiveState {
  const r = ref(size);
  return run(s, { type: "previewStarted", key, requestId: r.requestId, at: 0 }, { type: "previewLoaded", key, requestId: r.requestId, ref: r, url: `http://asset.localhost/${size}.jpg` });
}

function iueState(): ArchiveState {
  const iue = run(base(), { type: "missionSwitched", form: { ...base().form, mission: "iue", filters: {} }, generation: 1 });
  return run(
    iue,
    { type: "searchStarted", generation: 2, text: "HD 93129A", kind: "coordinates", name: null, query: { ...QUERY, mission: "iue" }, at: 0 },
    { type: "pageLoaded", generation: 2, page: page([row("SWP01234", { instrument: "SWP", levelClass: "unknown", level: null })], { mission: "iue" }) },
  );
}

function html<P extends object>(component: ComponentType<P>, props: P): string {
  return renderToStaticMarkup(createElement(component, props));
}

function tag(markup: string, testid: string, extra = ""): string {
  return new RegExp(`<[a-z]+[^>]*data-testid="${testid}"${extra}[^>]*>`).exec(markup)?.[0] ?? "";
}

function count(markup: string, testid: string): number {
  return markup.split(`data-testid="${testid}"`).length - 1;
}

function buttons(markup: string): string[] {
  return [...markup.matchAll(/<button[^>]*>([\s\S]*?)<\/button>/g)].map((m) => m[1].replace(/<[^>]+>/g, "").trim());
}

const netErr: MastError = { code: "mast_network", message: "connection refused", context: { operation: "search/jwst/api/v0.1/get_image_list", host: "mast.stsci.edu" } };

describe("detail pane", () => {
  it("renders the selected dataset with its header, tabs and filmstrip", () => {
    const markup = html(DetailPane, { state: withList(), ctl });
    const pane = tag(markup, "mast-detail");
    expect(pane).toContain(`data-dataset="${ID}"`);
    expect(pane).toContain('data-mode="pane"');
    expect(pane).toContain('role="complementary"');
    expect(pane).toContain(`aria-label="Details for ${ID}"`);
    expect(pane).toContain('data-mast-region="detail"');
    for (const word of ["JWST", "NIRCAM", "NRC_IMAGE", "L3", "PUBLIC"]) expect(markup).toContain(`>${word}<`);
    expect(markup).toContain("Programme 2739 · Obs 1 · M 16");
    expect(tag(markup, "mast-tab-previews")).toContain('aria-selected="true"');
    expect(tag(markup, "mast-tab-metadata")).toContain('aria-selected="false"');
    expect(markup).toContain(">Previews 6<");
    expect(count(markup, "mast-strip-item")).toBe(6);
    const files = [...markup.matchAll(/data-testid="mast-strip-item"[^>]*data-filename="([^"]+)"/g)].map((m) => m[1]);
    expect(files).toEqual(L3.entries.map((e) => e.previewFilename));
    for (const label of L3_FILTERS) expect(markup).toContain(`>${label}<`);
    expect(tag(markup, "mast-strip-item", ` data-filename="${F187N.previewFilename}"`)).toContain('aria-current="true"');
    expect(markup).toContain("2/6");
  });

  it("keeps a placeholder in the download stage until the preview arrives", () => {
    const markup = html(DetailPane, { state: withList(), ctl });
    const img = tag(markup, "mast-preview-img");
    expect(img).toContain('data-stage="download"');
    expect(img).toContain(`data-filename="${F187N.previewFilename}"`);
    expect(markup).toContain("Loading F187N preview");
  });

  it("shows the ready preview as an asset image with alt text and the quick WCS", () => {
    const markup = html(DetailPane, { state: ready(withList(), PANE, "pane"), ctl });
    const img = /<img[^>]*data-testid="mast-preview-img"[^>]*>/.exec(markup)?.[0] ?? "";
    expect(img).toContain('data-stage="ready"');
    expect(img).toContain(`data-filename="${F187N.previewFilename}"`);
    expect(img).toContain('src="http://asset.localhost/pane.jpg"');
    expect(img).toContain('alt="F187N i2d preview, 14341 by 8583 pixels, shown at 2048"');
    expect(markup).toContain("F187N i2d · 14341 x 8583 source");
    expect(markup).toContain("shown 2048 x 1226 · 5.8 MB");
    expect(markup).toContain("0.031&quot;/px · field 7.4&#x27; x 4.5&#x27;");
    expect(markup).toContain("8-bit quicklook made by MAST, not science data");
    const labels = buttons(markup);
    expect(labels.some((b) => b.startsWith("Large view"))).toBe(true);
    expect(labels).toContain("Copy MAST URI");
  });

  it("shows download progress with Cancel, then a failure with Retry inside the frame", () => {
    const r = ref("pane");
    const loading = run(
      withList(),
      { type: "previewStarted", key: PANE, requestId: r.requestId, at: 0 },
      { type: "previewProgress", requestId: r.requestId, received: 2_200_000, total: 5_800_000, stage: "download" },
    );
    const markup = html(DetailPane, { state: loading, ctl });
    expect(tag(markup, "mast-preview-img")).toContain('data-stage="download"');
    expect(markup).toContain("Downloading F187N preview");
    expect(markup).toContain("2.2 of 5.8 MB");
    expect(markup).toMatch(/role="progressbar"[^>]*aria-valuenow="38"/);
    expect(buttons(markup)).toContain("Cancel");
    const failed = run(loading, { type: "previewFailed", key: PANE, requestId: r.requestId, error: { code: "mast_no_image", message: "404", context: { status: 404, detail: "Image not found" } }, at: 3000 });
    const after = html(DetailPane, { state: failed, ctl });
    expect(tag(after, "mast-preview-img")).toContain('data-stage="failed"');
    expect(after).toContain("No preview for this image (MAST: Image not found).");
    expect(buttons(after).some((b) => b.startsWith("Retry"))).toBe(true);
  });

  it("says when the image list is still loading or could not be loaded", () => {
    const loading = run(searched(), { type: "imageListStarted", key: KEY, requestId: `g1:list:${ID}`, at: 0 });
    expect(html(DetailPane, { state: loading, ctl })).toContain(`Listing images for ${ID} …`);
    const failed = run(loading, { type: "imageListFailed", key: KEY, requestId: `g1:list:${ID}`, error: netErr, at: 1200 });
    const markup = html(DetailPane, { state: failed, ctl });
    expect(markup).toContain("Could not reach MAST (mast.stsci.edu): connection refused.");
    expect(buttons(markup).some((b) => b.startsWith("Retry"))).toBe(true);
  });

  describe("image slots in the detail show MAST's answer instead of the image", () => {
    const badGateway: MastError = { code: "mast_server", message: "HTTP 502", context: { status: 502, detail: "Bad Gateway", operation: "search/jwst/api/v0.1/get_image_preview" } };
    const slowThumb: MastError = { code: "mast_timeout", message: "no answer within 30 s", context: { timeoutSec: 30, operation: "search/jwst/api/v0.1/get_image_thumbnail" } };
    const listDown: MastError = { code: "mast_server", message: "HTTP 503", context: { status: 503, detail: "Service Unavailable", operation: "search/jwst/api/v0.1/get_image_list" } };

    it("puts the title, the HTTP status, MAST's text, Retry and Copy details in the preview stage", () => {
      const r = ref("pane");
      const s = run(withList(), { type: "previewStarted", key: PANE, requestId: r.requestId, at: 1000 }, { type: "previewFailed", key: PANE, requestId: r.requestId, error: badGateway, at: 42_000 });
      const markup = html(DetailPane, { state: s, ctl });
      const stage = tag(markup, "mast-preview-img");
      expect(stage).toContain('data-stage="failed"');
      expect(stage).toContain('data-error-code="mast_server"');
      expect(stage).toContain('data-error-status="502"');
      expect(stage).toContain('role="alert"');
      expect(markup).toContain(">MAST server error<");
      expect(markup).toContain("MAST had a problem answering (HTTP 502: Bad Gateway).");
      expect(markup).toContain("HTTP 502 · previews · waited 41 s");
      expect(markup).toMatch(/<blockquote[^>]*>Bad Gateway<\/blockquote>/);
      const labels = buttons(markup);
      expect(labels.some((b) => b.startsWith("Retry"))).toBe(true);
      expect(labels).toContain("Copy details");
      expect(markup).toContain("Code: mast_server");
      expect(markup).toContain("Waited: 41 s");
    });

    it("shows a failed strip thumbnail as a badge with the cause in its title, and an entry with no thumbnail as listed by MAST", () => {
      const first = L3.entries[0];
      const thumbKey = `${ID}:${first.thumbnailFilename}`;
      const noThumb = { ...L3, entries: L3.entries.map((e, i) => (i === 2 ? { ...e, thumbnailFilename: null } : e)) };
      const s = run(
        searched(),
        { type: "imageListLoaded", key: KEY, set: noThumb },
        { type: "thumbDemand", selected: [], visible: [thumbKey], ahead: [] },
        { type: "thumbsStarted", keys: [thumbKey], at: 0 },
        { type: "thumbFailed", key: thumbKey, error: slowThumb, at: 30_500 },
      );
      const markup = html(DetailPane, { state: s, ctl });
      const item = /<button[^>]*data-testid="mast-strip-item"[^>]*data-error-code="mast_timeout"[^>]*>[\s\S]*?<\/button>/.exec(markup)?.[0] ?? "";
      expect(item).toContain(`data-filename="${first.previewFilename}"`);
      expect(item).toMatch(/>30s</);
      expect(item).toMatch(/title="[^"]*MAST did not answer[^"]*"/);
      expect(item).toMatch(/title="[^"]*no answer within 30 s · waited 31 s[^"]*"/);
      const listed = tag(markup, "mast-strip-item", ` data-filename="${L3.entries[2].previewFilename}"`);
      expect(listed).toContain('data-thumb="none"');
      expect(listed).toContain("MAST lists no thumbnail for this image");
    });

    it("shows WAIT on strip thumbnails while MAST pauses requests", () => {
      const now = Date.now();
      const first = L3.entries[0];
      const thumbKey = `${ID}:${first.thumbnailFilename}`;
      const s = run(withList(), { type: "thumbDemand", selected: [], visible: [thumbKey], ahead: [] }, { type: "thumbsStarted", keys: [thumbKey], at: now }, { type: "thumbRateLimited", key: thumbKey, at: now, retryAfterSec: 30, consecutive: 1, status: 429 });
      const markup = html(DetailPane, { state: s, ctl });
      const item = tag(markup, "mast-strip-item", ` data-filename="${first.previewFilename}"`);
      expect(item).toContain('data-thumb="paused"');
      const body = /<button[^>]*data-testid="mast-strip-item"[^>]*data-thumb="paused"[^>]*>[\s\S]*?<\/button>/.exec(markup)?.[0] ?? "";
      expect(body).toMatch(/>WAIT</);
      expect(body).not.toContain("ab-mast-shimmer");
    });

    it("shows a failed image list as the same alert with the HTTP status and MAST's text", () => {
      const s = run(searched(), { type: "imageListStarted", key: KEY, requestId: `g1:list:${ID}`, at: 0 }, { type: "imageListFailed", key: KEY, requestId: `g1:list:${ID}`, error: listDown, at: 2500 });
      const markup = html(DetailPane, { state: s, ctl });
      const alert = /<div[^>]*role="alert"[^>]*data-error-code="mast_server"[^>]*>|<div[^>]*data-error-code="mast_server"[^>]*role="alert"[^>]*>/.exec(markup)?.[0] ?? "";
      expect(alert).toContain('data-error-status="503"');
      expect(markup).toContain("MAST had a problem answering (HTTP 503: Service Unavailable).");
      expect(markup).toContain("HTTP 503 · image lists · waited 2.5 s");
      expect(buttons(markup)).toContain("Copy details");
      expect(markup).not.toContain("MAST lists no preview");
    });
  });

  it("becomes a drawer in a narrow window, hidden and inert until it is opened", () => {
    const narrow = run(withList(), { type: "narrowSet", narrow: true });
    const closed = tag(html(DetailPane, { state: narrow, ctl }), "mast-detail");
    expect(closed).toContain('data-mode="drawer"');
    expect(closed).toContain('data-open="false"');
    expect(closed).toContain('inert=""');
    expect(closed).not.toContain("data-mast-region");
    const open = html(DetailPane, { state: run(narrow, { type: "drawerSet", open: true }), ctl });
    expect(tag(open, "mast-detail")).toContain('data-open="true"');
    expect(tag(open, "mast-detail")).not.toContain("inert");
    expect(open).toContain('aria-label="Close the detail"');
  });

  it("shows exclusive access in the header and explains why there are no previews", () => {
    const excl = row("jw01234-o001_t001_miri", { instrument: "MIRI", access: "PROPRIETARY", releaseDate: "2027-03-02T00:00:00" });
    const markup = html(DetailPane, { state: searched([excl]), ctl });
    expect(markup).toContain("EXCLUSIVE until 2027-03-02");
    expect(markup).toContain(EXCLUSIVE_PREVIEWS_TEXT);
    expect(count(markup, "mast-strip-item")).toBe(0);
    expect(count(markup, "mast-preview-img")).toBe(0);
  });

  it("disables Previews with the reason for a mission without images and opens on Metadata", () => {
    const markup = html(DetailPane, { state: iueState(), ctl });
    const previews = tag(markup, "mast-tab-previews");
    expect(previews).toContain('aria-disabled="true"');
    expect(previews).toContain("title=\"MAST&#x27;s IUE search API serves no thumbnails or previews.\"");
    expect(markup).toContain(">Previews (none)<");
    expect(tag(markup, "mast-tab-metadata")).toContain('aria-selected="true"');
  });

  it("lists identifiers, pointing, observation, query copies and every column in Metadata", () => {
    const markup = html(DetailPane, { state: run(withList(), { type: "detailTab", tab: "metadata" }), ctl });
    expect(tag(markup, "mast-tab-metadata")).toContain('aria-selected="true"');
    expect(markup).toContain(ID);
    expect(markup).toContain("2739 / 1");
    expect(markup).toContain('<span class="ab-mast-meta-note">Open an image in the workspace to draw footprints on it</span>');
    expect(markup).not.toContain("Draw it on the open image with Show on, above");
    expect(tag(markup, "mast-footprint-toggle")).toContain("disabled");
    expect(markup).toMatch(/data-testid="mast-footprint-reason"[^>]*>Open an image in the workspace to draw footprints on it</);
    expect(markup).toContain("Show on image");
    const labels = buttons(markup);
    for (const b of ["Copy request JSON", "Copy curl", "Copy row as TSV"]) expect(labels).toContain(b);
    expect(markup).toContain("All columns (6)");
    expect(markup).toMatch(/placeholder="Filter columns"/);
  });
});

describe("products tab", () => {
  const productsTab = (s: ArchiveState) => run(s, { type: "detailTab", tab: "products" });

  it("shows the listing in progress with the slow-listing note and Cancel", () => {
    const s = run(productsTab(withList()), { type: "productsStarted", key: KEY, requestId: `g1:products:${ID}`, at: 0 });
    const markup = html(DetailPane, { state: s, ctl });
    expect(tag(markup, "mast-products")).toContain('data-state="loading"');
    expect(markup).toContain(`Listing products for ${ID}`);
    expect(markup).toContain("MAST takes 20 to 30 s for listings it has not seen recently.");
    expect(buttons(markup)).toContain("Cancel");
  });

  it("groups the listing, counts hidden files and offers selection, downloads and copy actions", () => {
    const s = run(
      productsTab(withList()),
      { type: "productsStarted", key: KEY, requestId: `g1:products:${ID}`, at: 0 },
      { type: "productsLoaded", key: KEY, requestId: `g1:products:${ID}`, list: productList() },
    );
    const markup = html(DetailPane, { state: s, ctl });
    expect(tag(markup, "mast-products")).toContain('data-state="ready"');
    expect(markup).toContain(">Products 10<");
    expect(markup).toMatch(/data-group="image" data-count="2"/);
    expect(markup).toMatch(/data-group="catalog" data-count="1"/);
    const hidden = tag(markup, "mast-hidden-summary");
    expect(hidden).toContain('data-reference="2"');
    expect(hidden).toContain('data-guidestar="1"');
    expect(markup).toContain("2 CRDS reference files, 1 guide-star file");
    expect(markup).toContain("a_i2d.fits");
    expect(markup).toContain('title="5,420,000,000 bytes"');
    const downloadButtons = buttons(markup).filter((b) => /download/i.test(b));
    expect(downloadButtons).toEqual(["Download", "Download and open", "Copy Download/file URLs"]);
    expect(buttons(markup)).toContain("Copy MAST URIs");
    expect(tag(markup, "mast-dl-bar")).not.toBe("");
    expect(markup).toContain("Select files to download");
    expect(markup).toMatch(/<input type="checkbox" class="ab-mast-prow-pick" aria-label="Select a_i2d.fits"/);
    expect(markup).not.toContain("mast-dl-confirm");
    expect(markup).toContain("Downloads go to the mast folder of the Data folder (Settings)");
    expect(markup).not.toContain("arrives with the download engine");
  });

  it("lists Roman products without checkboxes or download buttons and says why", () => {
    const roman = MISSIONS.find((m) => m.id === "roman");
    if (!roman) throw new Error("the fixtures lost the Roman mission");
    const key = "roman:r0001";
    const s = run(
      base(),
      { type: "productsStarted", key, requestId: "g1:products:r0001", at: 0 },
      { type: "productsLoaded", key, requestId: "g1:products:r0001", list: productList() },
    );
    const markup = html(ProductsTab, { state: s, ctl, row: row("r0001"), mission: roman });
    expect(tag(markup, "mast-products")).toContain('data-state="ready"');
    expect(markup).toContain("a_i2d.fits");
    expect(markup).not.toContain("ab-mast-prow-pick");
    expect(tag(markup, "mast-dl-bar")).toContain('data-state="unavailable"');
    expect(markup).toContain("Roman downloads are not enabled yet.");
    expect(buttons(markup).filter((b) => /^Download/.test(b))).toEqual([]);
  });

  it("lists IUE products without checkboxes or download buttons and says why", () => {
    const iue = MISSIONS.find((m) => m.id === "iue");
    if (!iue) throw new Error("the fixtures lost the IUE mission");
    const key = "iue:lwp00501";
    const lwp = product("lwp00501.lihi.gz", "spectrum", 717_341, { datasetId: "lwp00501", mastUri: "mast:IUE/url//pub/iue/data/lwp/00000/lwp00501.lihi.gz" });
    const s = run(
      base(),
      { type: "productsStarted", key, requestId: "g1:products:lwp00501", at: 0 },
      { type: "productsLoaded", key, requestId: "g1:products:lwp00501", list: { ...productList(), mission: "iue", datasetIds: ["lwp00501"], products: [lwp] } },
    );
    const markup = html(ProductsTab, { state: s, ctl, row: row("lwp00501"), mission: { ...iue, capabilities: { ...iue.capabilities, products: true } } });
    expect(tag(markup, "mast-products")).toContain('data-state="ready"');
    expect(markup).not.toContain("ab-mast-prow-pick");
    expect(tag(markup, "mast-dl-bar")).toContain('data-state="unavailable"');
    expect(markup).toContain("IUE downloads are not enabled yet.");
    expect(buttons(markup).filter((b) => /^Download/.test(b))).toEqual([]);
  });

  it("disables the checkbox of a file the app cannot download and keeps the others", () => {
    const classy = product("hlsp_classy_hst_cos_j0021+0052_g130m_v1_coadded.fits", "image", 1_000, {
      mastUri: "mast:HLSP/classy/j0021+0052/hlsp_classy_hst_cos_j0021+0052_g130m_v1_coadded.fits",
    });
    const list = productList();
    const s = run(
      productsTab(withList()),
      { type: "productsStarted", key: KEY, requestId: `g1:products:${ID}`, at: 0 },
      { type: "productsLoaded", key: KEY, requestId: `g1:products:${ID}`, list: { ...list, products: [...list.products, classy] } },
    );
    const markup = html(DetailPane, { state: s, ctl });
    expect(markup).toMatch(/<input type="checkbox" class="ab-mast-prow-pick" aria-label="Select a_i2d.fits"/);
    const box = /<input[^>]*aria-label="Select hlsp_classy_hst_cos_j0021\+0052_g130m_v1_coadded.fits"[^>]*>/.exec(markup)?.[0] ?? "";
    expect(box).toContain('disabled=""');
    expect(box).toContain("title=\"Only HLSP files");
    expect(tag(markup, "mast-dl-bar")).not.toContain('data-state="unavailable"');
  });

  it("puts the error inside the tab with Retry", () => {
    const s = run(
      productsTab(withList()),
      { type: "productsStarted", key: KEY, requestId: `g1:products:${ID}`, at: 0 },
      { type: "productsFailed", key: KEY, requestId: `g1:products:${ID}`, error: { code: "mast_server", message: "x", context: { status: 504 } } },
    );
    const markup = html(DetailPane, { state: s, ctl });
    expect(tag(markup, "mast-products")).toContain('data-state="failed"');
    expect(markup).toContain("HTTP 504");
    expect(buttons(markup).some((b) => b.startsWith("Retry"))).toBe(true);
  });

  it("explains when a mission's product lists are not supported yet", () => {
    const markup = html(DetailPane, { state: productsTab(iueState()), ctl });
    expect(tag(markup, "mast-products")).toContain('data-state="unavailable"');
    expect(markup).toContain("Product lists for IUE arrive with spectra support (MA15)");
  });
});

describe("Large view", () => {
  function opened(s = withList()): ArchiveState {
    return run(s, { type: "largeOpened", datasetId: ID, imageKey: F187N.key });
  }

  it("renders nothing while closed", () => {
    expect(html(LargeView, { state: withList(), ctl })).toBe("");
  });

  it("is a modal dialog naming the dataset and image, with the 4096 px copy as its first image", () => {
    const markup = html(LargeView, { state: ready(ready(opened(), PANE, "pane"), LARGE, "large"), ctl });
    const box = tag(markup, "mast-lightbox");
    expect(box).toContain('role="dialog"');
    expect(box).toContain('aria-modal="true"');
    expect(box).toContain(`data-dataset="${ID}"`);
    expect(box).toContain(`data-filename="${F187N.previewFilename}"`);
    const first = /<img[^>]*>/.exec(markup.slice(markup.indexOf('data-testid="mast-lightbox"')))?.[0] ?? "";
    expect(first).toContain('src="http://asset.localhost/large.jpg"');
    expect(first).toContain('alt="F187N i2d preview, 14341 by 8583 pixels, shown at 4096"');
    expect(markup).toContain("dataset 1 of 2");
    expect(markup).toContain("source 14341 x 8583");
    expect(markup).toContain("shown 4096 x 2451");
    expect(markup).toContain("Programme 2739 · Obs 1");
    expect(markup).toContain("orientation not given");
    expect(markup).toContain("2/6");
    for (const label of ["Zoom in", "Zoom out", "Fit to the window", "Actual size of the 4096 px copy"]) expect(markup).toContain(`aria-label="${label}"`);
  });

  it("shows the pane copy blurred while the large copy loads", () => {
    const r = ref("large");
    const s = run(ready(opened(), PANE, "pane"), { type: "previewStarted", key: LARGE, requestId: r.requestId, at: 0 });
    const markup = html(LargeView, { state: s, ctl });
    expect(markup).toContain('src="http://asset.localhost/pane.jpg"');
    expect(markup).toContain("ab-mast-large-blur");
    expect(markup).toContain("Loading the 4096 px copy");
  });

  it("shows a failed image list and a failed pane copy as an alert with the HTTP status and MAST's text", () => {
    const down: MastError = { code: "mast_server", message: "HTTP 503", context: { status: 503, detail: "Service Unavailable", operation: "search/jwst/api/v0.1/get_image_list" } };
    const listFailed = run(
      opened(searched()),
      { type: "imageListStarted", key: KEY, requestId: `g1:list:${ID}`, at: 0 },
      { type: "imageListFailed", key: KEY, requestId: `g1:list:${ID}`, error: down, at: 1000 },
    );
    const list = html(LargeView, { state: listFailed, ctl });
    expect(list).toMatch(/class="ab-mast-large-status"[^>]*role="alert"|role="alert"[^>]*class="ab-mast-large-status"/);
    expect(list).toContain("HTTP 503");
    expect(list).toContain("Service Unavailable");
    expect(list).not.toMatch(/class="ab-mast-large-status"[^>]*role="status"/);
    const r = ref("pane");
    const paneFailed = run(opened(), { type: "previewStarted", key: PANE, requestId: r.requestId, at: 0 }, { type: "previewFailed", key: PANE, requestId: r.requestId, error: { ...down, context: { ...down.context, operation: "search/jwst/api/v0.1/get_image_preview" } }, at: 500 });
    const pane = html(LargeView, { state: paneFailed, ctl });
    expect(pane).toMatch(/role="alert"/);
    expect(pane).toContain("HTTP 503");
    expect(pane).toContain("Service Unavailable");
    expect(pane).toContain('data-error-code="mast_server"');
  });

  it("shows a compact chip with the HTTP status when the 4096 px copy fails", () => {
    const r = ref("large");
    const s = run(
      ready(opened(), PANE, "pane"),
      { type: "previewStarted", key: LARGE, requestId: r.requestId, at: 0 },
      { type: "previewFailed", key: LARGE, requestId: r.requestId, error: { code: "mast_server", message: "HTTP 503", context: { status: 503, detail: "Service Unavailable" } }, at: 800 },
    );
    const markup = html(LargeView, { state: s, ctl });
    const chip = /<div class="ab-mast-large-chip"[^>]*>[\s\S]*?<\/div>/.exec(markup)?.[0] ?? "";
    expect(chip).toContain("4096 px copy failed: HTTP 503 · Service Unavailable");
    expect(chip).toMatch(/title="[^"]*Code: mast_server[^"]*"/);
    expect(chip).toContain("Retry");
  });

  it("shows the pane copy sharp under the chip once the 4096 px copy has failed", () => {
    const r = ref("large");
    const s = run(
      ready(opened(), PANE, "pane"),
      { type: "previewStarted", key: LARGE, requestId: r.requestId, at: 0 },
      { type: "previewFailed", key: LARGE, requestId: r.requestId, error: { code: "mast_server", message: "HTTP 503", context: { status: 503, detail: "Service Unavailable" } }, at: 800 },
    );
    const markup = html(LargeView, { state: s, ctl });
    const img = /<img[^>]*src="http:\/\/asset\.localhost\/pane\.jpg"[^>]*>/.exec(markup)?.[0] ?? "";
    expect(img).not.toBe("");
    expect(img).not.toContain("ab-mast-large-blur");
    expect(img).not.toContain('aria-hidden="true"');
  });

  it("says it is listing images for a dataset reached with Up or Down", () => {
    const s = run(opened(), { type: "largeDataset", datasetId: "jw02739-o002_t001_miri" }, { type: "imageListStarted", key: "jwst:jw02739-o002_t001_miri", requestId: "g1:list:x", at: 0 });
    const markup = html(LargeView, { state: s, ctl });
    expect(tag(markup, "mast-lightbox")).toContain('data-dataset="jw02739-o002_t001_miri"');
    expect(markup).toContain("Listing images for jw02739-o002_t001_miri");
    expect(markup).toContain("dataset 2 of 2");
  });
});
