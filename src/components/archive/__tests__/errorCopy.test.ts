import { describe, expect, it } from "vitest";
import { CACHE_TROUBLE_TEXT, MISSING_FILE, errorDetails, errorView, pausedSlot, rateLimitText, slotError, tokenSaveFailureText } from "../logic/errorCopy";
import type { MastError, MastMissionId } from "../../../shared/types/mast";

const err = (code: MastError["code"], message = "", context: MastError["context"] = {}): MastError => ({ code, message, context });
const NOW = new Date(2026, 9, 16, 10, 42);

describe("errorView: search surface", () => {
  it("blocks on a network error with no results, with the cause and the kept query", () => {
    expect(errorView(err("mast_network", "connection refused", { host: "mast.stsci.edu" }), { surface: "search", hasResults: false })).toEqual({
      placement: "blocking",
      bannerKind: "network",
      title: "Could not reach MAST (mast.stsci.edu)",
      text: "The connection failed: connection refused. Your query is kept.",
      short: "network",
    });
  });

  it("shows a banner on a network error when results are on screen", () => {
    expect(errorView(err("mast_network", "connection refused.", { host: "mast.stsci.edu" }), { surface: "search", hasResults: true })).toMatchObject({
      placement: "banner",
      bannerKind: "network",
      text: "Could not reach MAST (mast.stsci.edu): connection refused.",
    });
  });

  it("quotes the timeout in seconds", () => {
    expect(errorView(err("mast_timeout", "timed out", { timeoutSec: 4 }), { surface: "search", hasResults: true }).text).toBe(
      "MAST did not answer within 4 s. First queries of the day can take over a minute; try again, or narrow the radius or add a filter.",
    );
    expect(errorView(err("mast_timeout", "timed out"), { surface: "search", hasResults: false })).toMatchObject({
      placement: "blocking",
      bannerKind: "timeout",
      text: "MAST did not answer within 120 s. First queries of the day can take over a minute; try again, or narrow the radius or add a filter.",
    });
  });

  it("blames the level condition and points to All levels when a JWST level search times out", () => {
    const text = "MAST did not answer within 120 s. MAST is sometimes slow with a level condition; All levels usually answers in a few seconds.";
    expect(errorView(err("mast_timeout", "timed out"), { surface: "search", hasResults: false, levelSearch: true })).toMatchObject({ placement: "blocking", bannerKind: "timeout", title: "MAST did not answer in time", text });
    expect(errorView(err("mast_timeout", "timed out", { timeoutSec: 4 }), { surface: "search", hasResults: true, levelSearch: true }).text).toBe(
      "MAST did not answer within 4 s. MAST is sometimes slow with a level condition; All levels usually answers in a few seconds.",
    );
    expect(errorView(err("mast_timeout", "x", { timeoutSec: 30 }), { surface: "thumb", levelSearch: true }).text).toBe("MAST did not send this thumbnail within 30 s. Try again in a moment.");
  });

  it("names the HTTP status of a server error", () => {
    expect(errorView(err("mast_server", "bad gateway", { status: 504 }), { surface: "search", hasResults: true })).toMatchObject({
      placement: "banner",
      bannerKind: "server",
      text: "MAST had a problem answering (HTTP 504). This is on MAST's side; try again in a moment.",
    });
    expect(errorView(err("mast_server", "x"), { surface: "search", hasResults: true }).text).toBe(
      "MAST had a problem answering. This is on MAST's side; try again in a moment.",
    );
  });

  it("puts a rejected query inline with MAST's detail and no doubled period", () => {
    expect(errorView(err("mast_rejected", "400", { status: 400, detail: "Radius greater than 30 arcminutes is not supported" }), { surface: "search" })).toMatchObject({
      placement: "inline",
      text: "MAST rejected the query: Radius greater than 30 arcminutes is not supported.",
    });
    expect(errorView(err("mast_rejected", "Bad value."), { surface: "search" }).text).toBe("MAST rejected the query: Bad value.");
  });

  it("puts local validation inline as Rust worded it", () => {
    expect(errorView(err("mast_bad_input", "Radius: must be between 0.1' and 30'"), { surface: "search" })).toMatchObject({
      placement: "inline",
      text: "Radius: must be between 0.1' and 30'",
    });
  });

  it("explains a decode failure", () => {
    expect(errorView(err("mast_decode", "unexpected response format", { operation: "search/jwst/api/v0.1/search" }), { surface: "search", hasResults: false })).toMatchObject({
      placement: "blocking",
      bannerKind: "decode",
      text: "MAST answered in a format AstroBurst does not understand. AstroBurst may need an update.",
    });
  });

  it("routes Roman authorization errors to the Roman panel and stays silent on cancel", () => {
    expect(errorView(err("mast_unauthorized", "x", { mission: "roman", status: 401 }), { surface: "search" }).placement).toBe("roman");
    expect(errorView(err("mast_token_rejected", "x"), { surface: "search" }).placement).toBe("roman");
    expect(errorView(err("mast_cancelled", "aborted"), { surface: "search" }).placement).toBe("silent");
  });

  it("marks an undeployed mission search unavailable with the check time", () => {
    expect(errorView(err("mast_not_deployed", "404", { mission: "tess", operation: "search/tess/api/v0.1/search", status: 404 }), { surface: "search", missionLabel: "TESS", now: NOW })).toMatchObject({
      placement: "unavailable",
      text: "MAST does not serve the TESS Mission Search API right now (checked 10:42).",
    });
  });

  it("names the mission by its label when the caller passes none", () => {
    const preview = (mission: MastMissionId) => err("mast_not_deployed", "404", { mission, operation: `search/${mission}/api/v0.1/get_image_preview`, status: 404 });
    expect(errorView(preview("roman"), { surface: "preview", now: NOW }).text).toBe("MAST does not serve the Roman previews right now (checked 10:42).");
    expect(errorView(preview("roman_cgi"), { surface: "preview", now: NOW }).text).toBe("MAST does not serve the Roman CGI previews right now (checked 10:42).");
    expect(errorView(preview("multi_mission"), { surface: "preview", now: NOW }).text).toBe("MAST does not serve the All missions previews right now (checked 10:42).");
  });

  it("shows the rate-limit copy for a search, which never retries on its own", () => {
    expect(errorView(err("mast_rate_limited", "429", { status: 429, retryAfterSec: 42, consecutive: 1 }), { surface: "search", hasResults: true })).toMatchObject({
      placement: "banner",
      bannerKind: "rate-limited",
      text: "MAST is limiting requests (HTTP 429). Retry in 42 s.",
    });
  });
});

describe("errorView: per-item surfaces", () => {
  it("gives short thumbnail reasons for the card tooltip", () => {
    expect(errorView(err("mast_not_image", "sniff"), { surface: "thumb" })).toMatchObject({ placement: "item", short: "not an image", text: "MAST returned something that is not an image." });
    expect(errorView(err("mast_network", "reset"), { surface: "thumb" })).toMatchObject({ placement: "item", short: "network" });
    expect(errorView(err("mast_no_image", "401", { status: 401, detail: "UNAUTHORIZED USER: anonymous" }), { surface: "thumb", rowPublic: false }).short).toBe("exclusive access");
    expect(errorView(err("mast_unauthorized", "401"), { surface: "thumb", rowPublic: false }).short).toBe("exclusive access");
  });

  it("words a missing preview with MAST's detail, or exclusive access for non-public rows", () => {
    expect(errorView(err("mast_no_image", "404", { status: 404, detail: "Image not found" }), { surface: "preview", rowPublic: true })).toMatchObject({
      placement: "item",
      text: "No preview for this image (MAST: Image not found).",
    });
    expect(errorView(err("mast_no_image", "401", { status: 401 }), { surface: "preview", rowPublic: false }).text).toBe("No preview for this image (exclusive access).");
  });

  it("gives the size of an image that is too large", () => {
    expect(errorView(err("mast_too_large", "cap", { bytes: 71_000_000 }), { surface: "preview" })).toMatchObject({
      placement: "item",
      short: "too large",
      text: "This image is larger than AstroBurst previews (71 MB). It will be available through downloads.",
    });
  });

  it("shows rate limiting on thumbnails as the pause banner", () => {
    expect(errorView(err("mast_rate_limited", "429", { status: 429, retryAfterSec: 5 }), { surface: "thumb" })).toMatchObject({
      placement: "banner",
      bannerKind: "rate-limited",
      text: "MAST is limiting requests (HTTP 429). Thumbnails are paused and resume in 5 s.",
    });
  });

  it("keeps product and list failures inside their tab", () => {
    expect(errorView(err("mast_server", "x", { status: 502 }), { surface: "products" })).toMatchObject({ placement: "item", bannerKind: "server" });
    expect(errorView(err("mast_timeout", "x", { timeoutSec: 60 }), { surface: "list" })).toMatchObject({ placement: "item", bannerKind: "timeout" });
  });

  it("raises a cache banner that says images wait for disk space or Clear MAST cache", () => {
    expect(errorView(err("mast_cache", "disk full"), { surface: "preview" })).toMatchObject({
      placement: "banner",
      bannerKind: "cache",
      short: "cache",
      text: "AstroBurst could not use its MAST cache (disk full). Images from MAST cannot be shown until it works again: free disk space or run Clear MAST cache, then retry.",
    });
    expect(errorView(err("mast_cache", ""), { surface: "thumb" }).text).toBe(CACHE_TROUBLE_TEXT);
    expect(CACHE_TROUBLE_TEXT).toBe("AstroBurst could not use its MAST cache. Images from MAST cannot be shown until it works again: free disk space or run Clear MAST cache, then retry.");
  });
});

describe("slotError: what an image slot shows instead of the image", () => {
  const THUMB_OP = "search/hst/api/v0.1/get_image_thumbnail";

  it("shows a server error with its HTTP status and MAST's own words", () => {
    const view = slotError(err("mast_server", "HTTP 500", { status: 500, detail: "Internal Server Error", operation: THUMB_OP }), { surface: "thumb", waitedMs: 2100 });
    expect(view).toMatchObject({ code: "mast_server", tone: "server", title: "MAST server error", meta: "HTTP 500", detail: "Internal Server Error", badge: "500", retry: true });
    expect(view?.lines).toEqual(["Code: mast_server", "HTTP status: 500", `Operation: ${THUMB_OP}`, "MAST detail: Internal Server Error", "Waited: 2.1 s"]);
  });

  it("shows a timeout with the limit, the tries and the time waited", () => {
    expect(slotError(err("mast_timeout", "no answer within 30 s", { timeoutSec: 30 }), { surface: "thumb", waitedMs: 41_000 })).toMatchObject({
      tone: "transient",
      title: "MAST did not answer",
      meta: "no answer within 30 s · waited 41 s",
      detail: null,
      badge: "30s",
      retry: true,
    });
    const twice = slotError(err("mast_timeout", "no answer within 4 s", { timeoutSec: 4, attempts: 2, elapsedMs: 9100, operation: THUMB_OP }), { surface: "thumb", waitedMs: 9400 });
    expect(twice?.meta).toBe("no answer within 4 s · 2 tries · waited 9.4 s");
    expect(twice?.badge).toBe("4s");
    expect(twice?.lines).toContain("Timeout: 4 s per request (thumbnails)");
    expect(twice?.lines).toContain("Attempts: 2");
    expect(twice?.lines).toContain("Waited: 9.4 s");
  });

  it("keeps 'No image at MAST' for a real missing image, and exclusive access for a row that is not public", () => {
    expect(slotError(err("mast_no_image", "Image not found", { status: 404, detail: "Image not found" }), { surface: "thumb" })).toMatchObject({
      tone: "muted",
      title: "No image at MAST",
      meta: "HTTP 404 · Image not found",
      detail: null,
      badge: "404",
      retry: true,
    });
    expect(slotError(err("mast_no_image", "no thumbnail listed for this dataset"), { surface: "thumb" })).toMatchObject({ meta: "no thumbnail listed for this dataset", badge: "none" });
    expect(slotError(err("mast_no_image", "401", { status: 401 }), { surface: "thumb", rowPublic: false })).toMatchObject({ title: "Exclusive access", meta: "HTTP 401", badge: "EXCL", retry: false });
    expect(slotError(err("mast_unauthorized", "authorization required", { status: 401 }), { surface: "preview" })).toMatchObject({ title: "Exclusive access", badge: "EXCL", retry: false });
  });

  it("names the host it could not reach, and only that host", () => {
    expect(slotError(err("mast_network", "connection refused", { host: "127.0.0.1:9" }), { surface: "thumb" })).toMatchObject({
      tone: "transient",
      title: "Could not reach MAST",
      meta: "127.0.0.1:9 · connection refused",
      badge: "NET",
      retry: true,
    });
    expect(slotError(err("mast_network", "connection refused"), { surface: "thumb" })?.meta).toBe("connection refused");
  });

  it("tells an answer MAST sent in an odd format from an error inside AstroBurst", () => {
    expect(slotError(err("mast_decode", "empty answer", { operation: "search/hst/api/v0.1/get_image_list" }), { surface: "list" })).toMatchObject({
      tone: "server",
      title: "Unexpected answer from MAST",
      meta: "image lists · empty answer",
      badge: "FMT",
    });
    expect(slotError(err("mast_decode", "invalid args `filename` for command"), { surface: "preview" })).toMatchObject({
      tone: "transient",
      title: "AstroBurst error",
      meta: "invalid args `filename` for command",
      badge: "ERR",
      retry: true,
    });
  });

  it("covers the remaining codes and gives no error view for a cancelled request", () => {
    expect(slotError(err("mast_rejected", "Bad filename", { status: 422, detail: "Bad filename" }), { surface: "preview" })).toMatchObject({ tone: "server", title: "MAST refused the request", meta: "HTTP 422", detail: "Bad filename", badge: "422" });
    expect(slotError(err("mast_not_image", "response is not an image", { operation: THUMB_OP }), { surface: "thumb" })).toMatchObject({ tone: "server", meta: "thumbnails", badge: "IMG" });
    expect(slotError(err("mast_too_large", "cap", { bytes: 74_000_000 }), { surface: "preview" })).toMatchObject({ tone: "muted", title: "Too large to preview", meta: "74 MB", badge: "BIG", retry: false });
    expect(slotError(err("mast_cache", "disk full"), { surface: "thumb" })).toMatchObject({ tone: "transient", meta: "disk full", badge: "DISK", retry: true });
    expect(slotError(MISSING_FILE, { surface: "thumb" })).toMatchObject({ title: "Cached file missing", badge: "DISK", retry: true });
    expect(slotError(err("mast_not_deployed", "404", { mission: "roman", operation: "search/roman/api/v0.1/get_image_preview", status: 404 }), { surface: "preview" })).toMatchObject({ tone: "muted", title: "Not served by MAST", meta: "Roman previews", badge: "N/A", retry: false });
    expect(slotError(err("mast_rate_limited", "429", { status: 429, retryAfterSec: 5 }), { surface: "thumb" })).toMatchObject({ title: "Paused by MAST", badge: "WAIT", retry: false });
    expect(slotError(err("mast_cancelled", "cancelled"), { surface: "thumb" })).toBeNull();
  });

  it("describes a paused queue with a live countdown, and the stop after MAST kept limiting", () => {
    expect(pausedSlot({ pausedUntil: 6000, status: 429, gaveUp: false }, 1000)).toMatchObject({ code: "mast_rate_limited", title: "Paused by MAST", meta: "HTTP 429 · resumes in 5 s", badge: "WAIT", retry: false });
    expect(pausedSlot({ pausedUntil: 6000, status: 429, gaveUp: true }, 60_000)).toMatchObject({ title: "Paused by MAST", meta: "HTTP 429 · stopped, press R to resume", badge: "WAIT" });
  });

  it("gives one line for the tooltip and screen readers", () => {
    const view = slotError(err("mast_server", "HTTP 503", { status: 503, detail: "Service Unavailable" }), { surface: "thumb" });
    expect(view?.summary).toBe("MAST server error, HTTP 503, Service Unavailable");
  });
});

describe("image timeouts and server errors name the request and quote MAST", () => {
  it("says which image request timed out and gives no search advice", () => {
    const thumb = errorView(err("mast_timeout", "x", { timeoutSec: 30, operation: "search/jwst/api/v0.1/get_image_thumbnail" }), { surface: "thumb" }).text;
    expect(thumb).toBe("MAST did not send this thumbnail within 30 s. Try again in a moment.");
    expect(thumb).not.toContain("radius");
    expect(errorView(err("mast_timeout", "x", { timeoutSec: 120, operation: "search/jwst/api/v0.1/get_image_preview" }), { surface: "preview" }).text).toBe("MAST did not send this preview within 120 s. Try again in a moment.");
    expect(errorView(err("mast_timeout", "x", { timeoutSec: 60, operation: "search/jwst/api/v0.1/get_image_list" }), { surface: "preview" }).text).toBe("MAST did not send the image list within 60 s. Try again in a moment.");
    expect(errorView(err("mast_timeout", "x", { timeoutSec: 60 }), { surface: "list" }).text).toBe("MAST did not send the image list within 60 s. Try again in a moment.");
    expect(errorView(err("mast_timeout", "x", { timeoutSec: 30, attempts: 2, operation: "search/hst/api/v0.1/get_image_thumbnail" }), { surface: "thumb" }).text).toBe("MAST did not send this thumbnail within 30 s (2 tries). Try again in a moment.");
  });

  it("quotes MAST's text in a server error", () => {
    expect(errorView(err("mast_server", "HTTP 500", { status: 500, detail: "Internal Server Error" }), { surface: "preview" }).text).toBe(
      "MAST had a problem answering (HTTP 500: Internal Server Error). This is on MAST's side; try again in a moment.",
    );
  });

  it("does not blame MAST for a fault inside AstroBurst, and says when MAST sent nothing", () => {
    expect(errorView(err("mast_decode", "boom"), { surface: "preview" }).text).toBe("AstroBurst hit an internal error: boom. This is not a MAST problem; try again, and report it if it keeps happening.");
    expect(errorView(err("mast_decode", "boom"), { surface: "search", hasResults: false })).toMatchObject({ placement: "blocking", title: "AstroBurst error" });
    expect(errorView(err("mast_decode", "empty answer", { operation: "search/hst/api/v0.1/get_image_list" }), { surface: "list" }).text).toBe("MAST sent an empty answer for the image lists. Try again in a moment.");
  });

  it("names the host that could not be reached, and no host when it is unknown", () => {
    expect(errorView(err("mast_network", "connection refused", { host: "127.0.0.1:9" }), { surface: "list" }).text).toBe("Could not reach MAST (127.0.0.1:9): connection refused.");
    expect(errorView(err("mast_network", "connection refused"), { surface: "list" }).text).toBe("Could not reach MAST: connection refused.");
    expect(errorView(err("mast_network", "refused", { host: "127.0.0.1:43123" }), { surface: "search", hasResults: false }).title).toBe("Could not reach MAST (127.0.0.1:43123)");
  });

  it("adds the host, the tries and the time MAST took to the details", () => {
    expect(errorDetails(err("mast_network", "x", { host: "127.0.0.1:9", attempts: 2, elapsedMs: 1500, operation: "search/jwst/api/v0.1/search" }))).toEqual([
      "Code: mast_network",
      "Operation: search/jwst/api/v0.1/search",
      "Host: 127.0.0.1:9",
      "Attempts: 2",
      "Time at MAST: 1.5 s",
    ]);
  });
});

describe("token copy", () => {
  it("says nothing was saved when MAST rejects the token", () => {
    expect(errorView(err("mast_token_rejected", "anon"), { surface: "token-save" })).toMatchObject({
      placement: "roman",
      text: "MAST did not accept this token (mistyped, expired or revoked). Nothing was saved.",
    });
  });

  it("tells a failed /info from a failed Roman check by whether the token was kept", () => {
    expect(tokenSaveFailureText(false)).toBe("Could not reach MAST to check the token. Nothing was saved.");
    expect(tokenSaveFailureText(true)).toBe("Token saved. MAST could not be reached to check Roman access.");
  });
});

describe("rateLimitText", () => {
  it("counts down for thumbnails and for a search", () => {
    expect(rateLimitText("thumbs", 42, 429)).toBe("MAST is limiting requests (HTTP 429). Thumbnails are paused and resume in 42 s.");
    expect(rateLimitText("search", 3, null)).toBe("MAST is limiting requests. Retry in 3 s.");
  });
});

describe("errorDetails", () => {
  it("lists code, status, operation and at most 300 characters of MAST's detail, never headers", () => {
    const lines = errorDetails(err("mast_rejected", "x", { status: 400, operation: "search/jwst/api/v0.1/search", detail: "d".repeat(400) }));
    expect(lines[0]).toBe("Code: mast_rejected");
    expect(lines[1]).toBe("HTTP status: 400");
    expect(lines[2]).toBe("Operation: search/jwst/api/v0.1/search");
    expect(lines[3]).toBe(`MAST detail: ${"d".repeat(300)}`);
    expect(lines).toHaveLength(4);
  });

  it("strips a query string from the operation", () => {
    expect(errorDetails(err("mast_server", "x", { operation: "search/jwst/api/v0.1/get_image_preview?filename=a" }))).toEqual([
      "Code: mast_server",
      "Operation: search/jwst/api/v0.1/get_image_preview",
    ]);
  });
});

describe("download error codes", () => {
  it("place each download refusal on its item with Rust's wording", () => {
    const cases: [MastError["code"], string, string][] = [
      ["mast_size_mismatch", "MAST listed 3145728 bytes, the server sent 3145729", "size mismatch"],
      ["mast_busy", "this file is already downloading", "busy"],
      ["mast_stale", "MAST has a newer version of this file", "newer at MAST"],
      ["mast_quota", "the MAST download quota (20 GB) would be exceeded", "quota"],
      ["mast_uri_refused", "Roman downloads are not enabled yet", "not downloadable"],
      ["mast_peek_incomplete", "no science header within the first 524288 bytes (7 ranges)", "header not reached"],
    ];
    for (const [code, message, short] of cases) {
      expect(errorView(err(code, message), { surface: "products" })).toEqual({ placement: "item", text: `${message}.`, short });
      const slot = slotError(err(code, message), { surface: "list" });
      expect(slot?.code).toBe(code);
      expect(slot?.meta).toBe(message);
      expect(slot?.retry).toBe(code === "mast_size_mismatch");
    }
  });
});
