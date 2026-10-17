import { describe, expect, it, vi } from "vitest";
import { createElement, type ComponentType } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import ResultsRegion from "../ResultsRegion";
import QueryBar from "../QueryBar";
import DetailPane from "../DetailPane";
import { MoreMenuList } from "../MoreMenu";
import { archiveReducer, createInitialState, type ArchiveAction, type ArchiveState } from "../state/archiveReducer";
import type { ArchiveController } from "../state/archiveController";
import { clockText, moreMenuView } from "../logic/missionState";
import { TOKEN_EXPIRY_TEXT } from "../logic/errorCopy";
import { MISSIONS, page, row } from "./fixtures";
import type { MastQuery } from "../../../shared/types/mast";

const IUE_QUERY: MastQuery = {
  mission: "iue",
  where: { kind: "position", raDeg: 160.9, decDeg: -59.5, radiusArcmin: 3, label: null },
  program: null,
  datasetId: null,
  filters: {},
  preset: null,
  sort: [],
  offset: 0,
  limit: 200,
};

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

function on(mission: MastQuery["mission"], state = base()): ArchiveState {
  return run(state, { type: "missionSwitched", form: { ...state.form, mission, filters: {} }, generation: state.generation + 1 });
}

function html<P extends object>(component: ComponentType<P>, props: P): string {
  return renderToStaticMarkup(createElement(component, props));
}

function tag(markup: string, testid: string, extra = ""): string {
  return new RegExp(`<[a-z]+[^>]*data-testid="${testid}"${extra}[^>]*>`).exec(markup)?.[0] ?? "";
}

function attrTag(markup: string, attr: string): string {
  return new RegExp(`<[a-z]+[^>]*${attr}[^>]*>`).exec(markup)?.[0] ?? "";
}

function region(state: ArchiveState): string {
  return html(ResultsRegion, { state, ctl, around: null });
}

describe("Roman panel", () => {
  it("replaces the results with the access panel and an empty token form when no token is stored", () => {
    const markup = region(on("roman"));
    expect(tag(markup, "mast-results")).toContain('data-state="locked"');
    expect(tag(markup, "mast-roman-panel")).toContain('data-state="none"');
    expect(markup).toContain("Only authorized users can make search API calls");
    expect(markup).toContain("(HTTP 401, checked 2026-10-16)");
    expect(markup).toContain("Roman access on that account, granted by STScI. A token alone is not enough.");
    expect(markup).toContain("Token on this computer: none");
    expect(tag(markup, "mast-token-open-page")).not.toBe("");
    expect(markup).toContain("Open the MAST token page");
    expect(markup).toContain("opens auth.mast.stsci.edu/token in your browser");
    const input = tag(markup, "mast-token-input");
    expect(input).toContain('type="password"');
    expect(input).toContain('autoComplete="off"');
    expect(input).toContain('spellCheck="false"');
    expect(input).not.toContain("value=");
    expect(tag(markup, "mast-token-submit")).not.toBe("");
    expect(markup).toContain("Check and save");
    expect(markup).toContain(TOKEN_EXPIRY_TEXT);
    expect(markup).toContain("The token is stored only on this computer");
  });

  it("shows the checks and Cancel with Esc while MAST is asked, without the form", () => {
    const markup = region(run(on("roman"), { type: "tokenCheckStarted", step: "info" }));
    expect(tag(markup, "mast-roman-panel")).toContain('data-state="checking"');
    expect(markup).toContain("Checking with MAST");
    expect(markup).toContain("Token accepted by auth.mast.stsci.edu");
    expect(markup).toMatch(/Cancel<kbd[^>]*>Esc<\/kbd>/);
    expect(markup).not.toContain('data-testid="mast-token-input"');
  });

  it("collapses to one line above the Roman results once access is confirmed", () => {
    const markup = region(run(on("roman"), { type: "tokenSaved", status: { present: true, authorized: true }, at: Date.now() }));
    expect(tag(markup, "mast-results")).not.toContain('data-state="locked"');
    expect(tag(markup, "mast-roman-panel")).toContain('data-state="authorized"');
    expect(markup).toContain("Roman access confirmed.");
    expect(markup).toContain("Manage token");
    expect(markup).not.toContain('data-testid="mast-token-input"');
  });

  it("explains a token without Roman access and offers Check again, Replace and Remove", () => {
    const markup = region(run(on("roman"), { type: "tokenSaved", status: { present: true, authorized: false }, at: Date.now() }));
    expect(tag(markup, "mast-roman-panel")).toContain('data-state="not-authorized"');
    expect(markup).toContain("does not have Roman access");
    expect(markup).toContain("Check again");
    expect(markup).toContain("Replace token");
    expect(markup).toContain("Remove token");
    expect(markup).not.toContain('data-testid="mast-token-input"');
  });

  it("keeps the form after a rejected token, marks the field invalid and says nothing was saved", () => {
    const markup = region(
      run(on("roman"), { type: "tokenCheckStarted", step: "info" }, { type: "tokenSaveFailed", error: { code: "mast_token_rejected", message: "", context: {} }, status: { present: false, authorized: null } }),
    );
    expect(tag(markup, "mast-roman-panel")).toContain('data-state="rejected"');
    expect(markup).toContain("Nothing was saved");
    const input = tag(markup, "mast-token-input");
    expect(input).toContain('aria-invalid="true"');
    expect(input).toMatch(/aria-describedby="[^"]*mast-token-outcome/);
  });

  it("says an expired token is no longer accepted, with its lifetime", () => {
    const markup = region(
      run(on("roman", run(base(), { type: "tokenStatus", status: { present: true, authorized: true } })), { type: "tokenCheckFailed", error: { code: "mast_token_rejected", message: "", context: {} } }),
    );
    expect(tag(markup, "mast-roman-panel")).toContain('data-state="expired"');
    expect(markup).toContain("no longer accepted by MAST");
    expect(markup).toContain("10 days");
    expect(markup).toContain("60 days");
  });

  it("says a check could not reach MAST and keeps the saved token", () => {
    const markup = region(
      run(on("roman"), { type: "tokenCheckStarted", step: "info" }, { type: "tokenSaveFailed", error: { code: "mast_network", message: "refused", context: {} }, status: { present: true, authorized: null } }),
    );
    expect(tag(markup, "mast-roman-panel")).toContain('data-state="unreachable"');
    expect(markup).toContain("Token saved. MAST could not be reached to check Roman access.");
    expect(markup).toContain("Check again");
  });

  it("shows the Roman status in the query bar and the reason Search is off", () => {
    const markup = html(QueryBar, { state: on("roman"), ctl });
    expect(markup).toContain("Search disabled: Roman needs an authorized MAST token");
    expect(tag(markup, "mast-search")).toContain("disabled");
  });
});

describe("More menu", () => {
  it("has a More button after the mission tabs", () => {
    const markup = html(QueryBar, { state: base(), ctl });
    const more = tag(markup, "mast-more");
    expect(more).toContain('aria-expanded="false"');
    expect(more).not.toContain("aria-controls");
    expect(markup.indexOf('data-testid="mast-more"')).toBeGreaterThan(markup.lastIndexOf('data-testid="mast-mission-tab"'));
  });

  it("lists IUE, CLASSY and ULLYSES enabled and the others disabled, each with its reason", () => {
    const markup = html(MoreMenuList, { view: moreMenuView(MISSIONS, "locked", {}, "jwst", Date.now()), onPick: vi.fn(), onCheck: vi.fn() });
    expect(tag(markup, "mast-more-menu")).not.toBe("");
    for (const id of ["iue", "classy", "ullyses"]) expect(attrTag(markup, `data-mission="${id}"`)).toContain('aria-disabled="false"');
    for (const id of ["roman_cgi", "roman_spectra", "tess", "multi_mission"]) expect(attrTag(markup, `data-mission="${id}"`)).toContain('aria-disabled="true"');
    expect(markup).toContain("search and metadata; no preview images at MAST");
    expect(markup).toContain("needs Roman authorization; coronagraphy is not processed by AstroBurst");
    expect(markup.split("not deployed by MAST (HTTP 404, checked 2026-10-16)").length - 1).toBe(3);
    expect(markup.split("data-check-mission=").length - 1).toBe(3);
    expect(markup).toContain('role="separator"');
  });

  it("shows a check in flight and the outcome of the last one", () => {
    const at = Date.now();
    const view = moreMenuView(MISSIONS, "locked", { tess: { stage: "checking", at, error: null }, multi_mission: { stage: "done", at, error: null } }, "jwst", at);
    const markup = html(MoreMenuList, { view, onPick: vi.fn(), onCheck: vi.fn() });
    expect(attrTag(markup, 'data-check-mission="tess"')).toContain('aria-disabled="true"');
    expect(markup).toContain("Checking");
    expect(markup).toContain(`not deployed by MAST (HTTP 404, checked ${clockText(at)} today)`);
  });
});

describe("missions that are not deployed or have no images", () => {
  it("shows the not-deployed panel with the time of the last check, Check again and Back to JWST", () => {
    const at = Date.now();
    const s = run(on("tess"), { type: "missionProbeStarted", mission: "tess", at }, { type: "missionProbed", mission: MISSIONS.find((m) => m.id === "tess") ?? MISSIONS[0], at });
    const markup = region(s);
    expect(tag(markup, "mast-results")).toContain('data-state="unavailable"');
    expect(markup).toContain("MAST does not serve the TESS Mission Search API right now");
    expect(markup).toContain(`Checked ${clockText(at)} today (HTTP 404). AstroBurst will show it once MAST deploys it.`);
    expect(tag(markup, "mast-check-again")).toContain('data-checking="false"');
    expect(markup).toContain("Back to JWST");
  });

  it("marks Check again busy while the probe runs", () => {
    const markup = region(run(on("tess"), { type: "missionProbeStarted", mission: "tess", at: Date.now() }));
    const check = tag(markup, "mast-check-again");
    expect(check).toContain('data-checking="true"');
    expect(check).toContain('aria-disabled="true"');
  });

  it("says in the detail that an image-less mission has no previews", () => {
    const s = run(
      on("iue"),
      { type: "searchStarted", generation: 2, text: "160.9 -59.5", kind: "coordinates", name: null, query: IUE_QUERY, at: 0 },
      { type: "pageLoaded", generation: 2, page: page([row("SWP01234", { levelClass: "unknown", level: null, instrument: "SWP" })], { mission: "iue" }) },
    );
    const markup = html(DetailPane, { state: s, ctl });
    expect(tag(markup, "mast-tab-previews")).toContain('aria-disabled="true"');
    expect(tag(markup, "mast-no-previews")).not.toBe("");
    expect(markup).toContain("MAST&#x27;s IUE search API serves no thumbnails or previews.");
  });
});
