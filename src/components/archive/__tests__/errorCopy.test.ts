import { describe, expect, it } from "vitest";
import { errorDetails, errorView, rateLimitText, tokenSaveFailureText } from "../logic/errorCopy";
import type { MastError } from "../../../shared/types/mast";

const err = (code: MastError["code"], message = "", context: MastError["context"] = {}): MastError => ({ code, message, context });
const NOW = new Date(2026, 9, 16, 10, 42);

describe("errorView: search surface", () => {
  it("blocks on a network error with no results, with the cause and the kept query", () => {
    expect(errorView(err("mast_network", "connection refused"), { surface: "search", hasResults: false })).toEqual({
      placement: "blocking",
      bannerKind: "network",
      title: "Could not reach MAST (mast.stsci.edu)",
      text: "The connection failed: connection refused. Your query is kept.",
      short: "network",
    });
  });

  it("shows a banner on a network error when results are on screen", () => {
    expect(errorView(err("mast_network", "connection refused."), { surface: "search", hasResults: true })).toMatchObject({
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
    expect(errorView(err("mast_decode", "missing field"), { surface: "search", hasResults: false })).toMatchObject({
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

  it("raises a cache banner", () => {
    expect(errorView(err("mast_cache", "disk full"), { surface: "preview" })).toMatchObject({
      placement: "banner",
      bannerKind: "cache",
      text: "AstroBurst could not write its MAST cache (disk full). Previews still load but are not kept.",
    });
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
