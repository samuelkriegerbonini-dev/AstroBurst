import { describe, expect, it } from "vitest";
import {
  ROMAN_SEARCH_DISABLED,
  checkedText,
  clockText,
  missionEntries,
  moreMenu,
  moreMenuView,
  notDeployedView,
  resultsGate,
  romanLock,
  tabMissions,
  viewAvailability,
} from "../logic/missionState";
import { MISSIONS, missionById } from "./fixtures";
import type { MastError, MastMission } from "../../../shared/types/mast";

describe("romanLock", () => {
  it("is locked without a token, or with a token MAST has not confirmed", () => {
    expect(romanLock({ status: null, checking: false, expired: false })).toBe("locked");
    expect(romanLock({ status: { present: false, authorized: null }, checking: false, expired: false })).toBe("locked");
    expect(romanLock({ status: { present: true, authorized: false }, checking: false, expired: false })).toBe("locked");
    expect(romanLock({ status: { present: true, authorized: null }, checking: false, expired: false })).toBe("locked");
  });

  it("is checking while a check runs, unlocked once authorized, expired after a rejected recheck", () => {
    expect(romanLock({ status: { present: true, authorized: null }, checking: true, expired: false })).toBe("checking");
    expect(romanLock({ status: { present: true, authorized: true }, checking: false, expired: false })).toBe("unlocked");
    expect(romanLock({ status: { present: true, authorized: true }, checking: false, expired: true })).toBe("expired");
  });
});

describe("missionEntries", () => {
  const entries = missionEntries(MISSIONS, "locked");
  const byId = Object.fromEntries(entries.map((e) => [e.id, e]));

  it("opens JWST and HST as tabs", () => {
    expect(byId.jwst).toMatchObject({ placement: "tab", enabled: true, reason: null, accent: "amber" });
    expect(byId.hst).toMatchObject({ placement: "tab", enabled: true, reason: null, accent: "sky" });
  });

  it("keeps Roman selectable but locked, with a violet accent", () => {
    expect(byId.roman).toMatchObject({ placement: "tab", enabled: true, lock: "locked", accent: "violet" });
    expect(missionEntries(MISSIONS, "unlocked").find((e) => e.id === "roman")?.lock).toBe("unlocked");
    expect(missionEntries(MISSIONS, "checking").find((e) => e.id === "roman")?.lock).toBe("checking");
    expect(missionEntries(MISSIONS, "expired").find((e) => e.id === "roman")?.lock).toBe("expired");
  });

  it("puts IUE, CLASSY and ULLYSES in More, enabled, with what they offer", () => {
    expect(byId.iue).toMatchObject({ placement: "more", enabled: true, reason: "search and metadata; no preview images at MAST" });
    expect(byId.classy).toMatchObject({ placement: "more", enabled: true, reason: "search and metadata (galaxy sample); no previews" });
    expect(byId.ullyses).toMatchObject({ placement: "more", enabled: true, reason: "search and metadata (targets); no previews" });
  });

  it("disables Roman CGI, Roman Spectra, TESS and All missions with their reasons", () => {
    expect(byId.roman_cgi).toMatchObject({ placement: "more", enabled: false, reason: "needs Roman authorization; coronagraphy is not processed by AstroBurst" });
    for (const id of ["roman_spectra", "tess", "multi_mission"]) {
      expect(byId[id]).toMatchObject({ placement: "more", enabled: false, reason: "not deployed by MAST (HTTP 404, checked 2026-10-16)" });
    }
  });

  it("enables an undeployed mission once a probe finds it open", () => {
    const probed: MastMission[] = MISSIONS.map((m) => (m.id === "tess" ? { ...m, availability: "open", capabilities: { search: true, images: false, products: false } } : m));
    expect(missionEntries(probed, "locked").find((e) => e.id === "tess")).toMatchObject({ enabled: true, reason: null });
  });

  it("uses the probe date in the reason", () => {
    const probed = MISSIONS.map((m) => (m.id === "tess" ? { ...m, checkedOn: "2026-11-02" } : m));
    expect(missionEntries(probed, "locked").find((e) => e.id === "tess")?.reason).toBe("not deployed by MAST (HTTP 404, checked 2026-11-02)");
  });
});

describe("tabMissions and moreMenu", () => {
  const entries = missionEntries(MISSIONS, "locked");

  it("shows JWST, HST and Roman, plus the current More mission as a fourth tab", () => {
    expect(tabMissions(entries, "jwst").map((e) => e.id)).toEqual(["jwst", "hst", "roman"]);
    expect(tabMissions(entries, "iue").map((e) => e.id)).toEqual(["jwst", "hst", "roman", "iue"]);
  });

  it("lists enabled More missions first, then the disabled ones", () => {
    const menu = moreMenu(entries);
    expect(menu.enabled.map((e) => e.id)).toEqual(["iue", "classy", "ullyses"]);
    expect(menu.disabled.map((e) => e.id)).toEqual(["roman_cgi", "roman_spectra", "tess", "multi_mission"]);
  });
});

describe("viewAvailability", () => {
  it("offers gallery, previews and products for JWST and HST", () => {
    expect(viewAvailability(missionById("jwst"))).toEqual({
      gallery: true,
      galleryReason: null,
      previews: true,
      previewsReason: null,
      products: true,
      productsReason: null,
    });
  });

  it("makes IUE table only, with Previews and Products disabled and their reasons", () => {
    expect(viewAvailability(missionById("iue"))).toEqual({
      gallery: false,
      galleryReason: "MAST's IUE search API serves no thumbnails or previews.",
      previews: false,
      previewsReason: "MAST's IUE search API serves no thumbnails or previews.",
      products: false,
      productsReason: "Product lists for IUE arrive with spectra support (MA15)",
    });
  });
});

describe("resultsGate", () => {
  it("locks Roman until access is confirmed, with the Search hint", () => {
    expect(resultsGate(missionById("roman"), "locked")).toBe("locked");
    expect(resultsGate(missionById("roman"), "checking")).toBe("locked");
    expect(resultsGate(missionById("roman"), "expired")).toBe("locked");
    expect(resultsGate(missionById("roman"), "unlocked")).toBe("open");
    expect(ROMAN_SEARCH_DISABLED).toBe("Roman needs an authorized MAST token");
  });

  it("marks undeployed missions unavailable and image-less missions as table missions", () => {
    expect(resultsGate(missionById("tess"), "locked")).toBe("unavailable");
    expect(resultsGate(missionById("iue"), "locked")).toBe("open");
    expect(resultsGate(missionById("jwst"), "locked")).toBe("open");
  });
});

describe("check times", () => {
  const now = new Date(2026, 9, 16, 15, 0).getTime();

  it("shows a check made today as a clock time and an older one with its date", () => {
    expect(clockText(new Date(2026, 9, 16, 9, 5).getTime())).toBe("09:05");
    expect(checkedText(new Date(2026, 9, 16, 10, 42).getTime(), now)).toBe("10:42 today");
    expect(checkedText(new Date(2026, 9, 15, 23, 59).getTime(), now)).toBe("2026-10-15 23:59");
  });
});

describe("moreMenuView", () => {
  const now = new Date(2026, 9, 16, 15, 0).getTime();
  const at = new Date(2026, 9, 16, 10, 42).getTime();
  const byId = (view: ReturnType<typeof moreMenuView>) => Object.fromEntries([...view.enabled, ...view.disabled].map((e) => [e.id, e]));

  it("lists the offers first and the disabled missions with their reasons, and offers Check again only where MAST may deploy later", () => {
    const view = moreMenuView(MISSIONS, "locked", {}, "jwst", now);
    expect(view.enabled.map((e) => [e.id, e.reason, e.check])).toEqual([
      ["iue", "search and metadata; no preview images at MAST", null],
      ["classy", "search and metadata (galaxy sample); no previews", null],
      ["ullyses", "search and metadata (targets); no previews", null],
    ]);
    expect(view.disabled.map((e) => [e.id, e.check])).toEqual([
      ["roman_cgi", null],
      ["roman_spectra", "idle"],
      ["tess", "idle"],
      ["multi_mission", "idle"],
    ]);
    expect(byId(view).tess.reason).toBe("not deployed by MAST (HTTP 404, checked 2026-10-16)");
    expect(byId(view).tess.note).toBeNull();
  });

  it("marks the current mission", () => {
    const view = moreMenuView(MISSIONS, "locked", {}, "classy", now);
    expect(byId(view).classy.current).toBe(true);
    expect(byId(view).iue.current).toBe(false);
  });

  it("shows a check in flight, the time of a finished check and why a check failed", () => {
    const network: MastError = { code: "mast_network", message: "connection refused", context: {} };
    const view = moreMenuView(
      MISSIONS,
      "locked",
      { tess: { stage: "checking", at, error: null }, multi_mission: { stage: "done", at, error: null }, roman_spectra: { stage: "failed", at, error: network } },
      "jwst",
      now,
    );
    const entries = byId(view);
    expect(entries.tess.check).toBe("checking");
    expect(entries.multi_mission).toMatchObject({ check: "idle", reason: "not deployed by MAST (HTTP 404, checked 10:42 today)", note: null });
    expect(entries.roman_spectra).toMatchObject({ check: "idle", reason: "not deployed by MAST (HTTP 404, checked 2026-10-16)", note: "Could not check again (network)" });
  });

  it("moves a mission MAST serves again to the offers", () => {
    const probed: MastMission[] = MISSIONS.map((m) => (m.id === "tess" ? { ...m, availability: "open", capabilities: { search: true, images: false, products: false } } : m));
    const view = moreMenuView(probed, "locked", { tess: { stage: "done", at, error: null } }, "jwst", now);
    expect(view.enabled.map((e) => e.id)).toEqual(["iue", "classy", "ullyses", "tess"]);
    expect(byId(view).tess.check).toBeNull();
  });
});

describe("notDeployedView", () => {
  const now = new Date(2026, 9, 16, 15, 0).getTime();

  it("names the mission and the API MAST does not serve, with the time of the last check", () => {
    expect(notDeployedView({ label: "TESS", operation: "search/tess/api/v0.1/search", at: new Date(2026, 9, 16, 10, 42).getTime(), status: 404, checkedOn: "2026-10-16", now })).toEqual({
      title: "MAST does not serve the TESS Mission Search API right now",
      text: "Checked 10:42 today (HTTP 404). AstroBurst will show it once MAST deploys it.",
    });
  });

  it("falls back to the date of the capability matrix and to the Mission Search API", () => {
    expect(notDeployedView({ label: "Roman Spectra", at: null, status: null, checkedOn: "2026-10-16", now })).toEqual({
      title: "MAST does not serve the Roman Spectra Mission Search API right now",
      text: "Checked 2026-10-16 (HTTP 404). AstroBurst will show it once MAST deploys it.",
    });
  });
});
