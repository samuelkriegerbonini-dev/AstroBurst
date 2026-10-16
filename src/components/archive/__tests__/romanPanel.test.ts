import { describe, expect, it } from "vitest";
import {
  ACTION_LABELS,
  CHECK_STEPS,
  CHECKING_TITLE,
  NOT_CHECKED_TEXT,
  ROMAN_NEEDS,
  ROMAN_PANEL_TITLE,
  TOKEN_PAGE_NOTE,
  TOKEN_STORAGE_NOTE,
  romanIntro,
  romanPanelView,
  tokenWasSaved,
  type RomanPanelInput,
} from "../logic/romanPanel";
import { MAST_TOKEN_PAGE_URL } from "../logic/tokenFormat";
import { ROMAN_NO_ACCESS_TEXT, TOKEN_EXPIRY_TEXT, TOKEN_REJECTED_TEXT } from "../logic/errorCopy";
import type { MastError } from "../../../shared/types/mast";

const AT = new Date(2026, 9, 16, 14, 2).getTime();

function input(over: Partial<RomanPanelInput> = {}): RomanPanelInput {
  return { state: "none", present: false, message: null, checkedAt: null, replacing: false, expanded: false, checkedOn: "2026-10-16", ...over };
}

describe("Roman panel copy", () => {
  it("quotes MAST's live answer with the date it was checked", () => {
    expect(ROMAN_PANEL_TITLE).toBe("Roman access");
    expect(romanIntro("2026-10-16")).toBe(
      'MAST answers Roman searches only for authorized users. Without a token MAST replies "Only authorized users can make search API calls" (HTTP 401, checked 2026-10-16).',
    );
    expect(ROMAN_NEEDS).toEqual([
      "A MAST API token, created on auth.mast.stsci.edu after signing in with your MyST account.",
      "Roman access on that account, granted by STScI. A token alone is not enough.",
    ]);
    expect(TOKEN_PAGE_NOTE).toBe("opens auth.mast.stsci.edu/token in your browser");
    expect(TOKEN_STORAGE_NOTE).toBe(
      "The token is stored only on this computer, sent only to mast.stsci.edu for Roman requests and to auth.mast.stsci.edu to check it, and never shown again.",
    );
    expect(MAST_TOKEN_PAGE_URL).toBe("https://auth.mast.stsci.edu/token?suggested_name=AstroBurst&suggested_scope=mast:exclusive_access");
  });

  it("labels every action", () => {
    expect(ACTION_LABELS).toEqual({
      "open-page": "Open the MAST token page",
      check: "Check again",
      replace: "Replace token",
      remove: "Remove token",
      cancel: "Cancel",
      "try-again": "Try again",
      manage: "Manage token",
      collapse: "Done",
      "cancel-replace": "Keep the saved token",
    });
  });
});

describe("romanPanelView", () => {
  it("explains what is needed and offers the token page and the form when no token is stored", () => {
    expect(romanPanelView(input())).toEqual({
      layout: "full",
      title: "Roman access",
      intro: romanIntro("2026-10-16"),
      needs: ROMAN_NEEDS,
      tokenLine: "Token on this computer: none",
      message: null,
      tone: null,
      checking: null,
      form: true,
      actions: ["open-page"],
    });
  });

  it("shows the two checks and only Cancel while MAST is asked", () => {
    const view = romanPanelView(input({ state: "checking" }));
    expect(view).toMatchObject({ layout: "full", tokenLine: null, form: false, actions: ["cancel"], checking: CHECK_STEPS });
    expect(CHECKING_TITLE).toBe("Checking with MAST");
    expect(CHECK_STEPS).toEqual(["Token accepted by auth.mast.stsci.edu", "Roman access"]);
  });

  it("collapses to one line once Roman access is confirmed, and expands to manage the token", () => {
    expect(romanPanelView(input({ state: "authorized", present: true, checkedAt: AT }))).toMatchObject({
      layout: "compact",
      message: "Roman access confirmed.",
      tone: "ok",
      form: false,
      actions: ["manage"],
    });
    expect(romanPanelView(input({ state: "authorized", present: true, checkedAt: AT, expanded: true }))).toMatchObject({
      layout: "full",
      intro: null,
      needs: null,
      tokenLine: "Token on this computer: saved, Roman access confirmed (checked 14:02)",
      actions: ["check", "replace", "remove", "collapse"],
    });
  });

  it("says a working token without Roman access is saved and how access is granted", () => {
    const view = romanPanelView(input({ state: "not-authorized", present: true, checkedAt: AT }));
    expect(view).toMatchObject({
      layout: "full",
      needs: null,
      tokenLine: "Token on this computer: saved, accepted by MAST, no Roman access (checked 14:02)",
      message: ROMAN_NO_ACCESS_TEXT,
      tone: "warn",
      form: false,
      actions: ["check", "replace", "remove"],
    });
    expect(view.message).toContain("does not have Roman access");
    expect(romanPanelView(input({ state: "not-authorized", present: true })).tokenLine).toBe("Token on this computer: saved, accepted by MAST, no Roman access");
  });

  it("opens the form to replace a saved token and offers to keep the saved one", () => {
    expect(romanPanelView(input({ state: "not-authorized", present: true, replacing: true }))).toMatchObject({
      form: true,
      actions: ["check", "remove", "cancel-replace"],
    });
    expect(romanPanelView(input({ state: "authorized", present: true, expanded: true, replacing: true }))).toMatchObject({
      layout: "full",
      form: true,
      actions: ["check", "remove", "cancel-replace"],
    });
  });

  it("keeps the form after a rejected token and says nothing was saved", () => {
    expect(romanPanelView(input({ state: "rejected", message: TOKEN_REJECTED_TEXT }))).toMatchObject({
      tokenLine: "Token on this computer: none",
      message: "MAST did not accept this token (mistyped, expired or revoked). Nothing was saved.",
      tone: "error",
      form: true,
      actions: ["open-page"],
    });
    expect(romanPanelView(input({ state: "rejected", present: true, message: TOKEN_REJECTED_TEXT }))).toMatchObject({
      tokenLine: "Token on this computer: saved, unchanged",
      form: true,
      actions: ["open-page", "remove"],
    });
  });

  it("says an expired token is no longer accepted, with the token lifetime", () => {
    const view = romanPanelView(input({ state: "expired", present: true }));
    expect(view).toMatchObject({
      tokenLine: "Token on this computer: saved, no longer accepted by MAST",
      message: TOKEN_EXPIRY_TEXT,
      tone: "warn",
      form: false,
      actions: ["open-page", "replace", "remove"],
    });
    expect(view.message).toContain("10 days");
    expect(view.message).toContain("60 days");
  });

  it("tells a saved token whose check could not reach MAST from one that was never saved", () => {
    expect(romanPanelView(input({ state: "unreachable", present: true, message: "Token saved. MAST could not be reached to check Roman access." }))).toMatchObject({
      tokenLine: "Token on this computer: saved, Roman access not checked",
      message: "Token saved. MAST could not be reached to check Roman access.",
      form: false,
      actions: ["check", "replace", "remove"],
    });
    expect(romanPanelView(input({ state: "unreachable", message: "Could not reach MAST to check the token. Nothing was saved." }))).toMatchObject({
      tokenLine: "Token on this computer: none",
      message: "Could not reach MAST to check the token. Nothing was saved.",
      tone: "error",
      form: true,
      actions: ["try-again", "open-page"],
    });
    expect(romanPanelView(input({ state: "unreachable", present: true })).message).toBe(NOT_CHECKED_TEXT);
    expect(NOT_CHECKED_TEXT).toBe("AstroBurst has not checked Roman access with MAST in this session.");
  });
});

describe("tokenWasSaved", () => {
  const net = (operation?: string): MastError => ({ code: "mast_network", message: "connection refused", context: operation ? { operation } : {} });

  it("knows the token was saved when the Roman check failed and was not when /info failed", () => {
    expect(tokenWasSaved(net("search/roman/api/v0.1/auth_check"), { present: true, authorized: true }, { present: true, authorized: null })).toBe(true);
    expect(tokenWasSaved(net("info"), { present: false, authorized: null }, { present: false, authorized: null })).toBe(false);
  });

  it("falls back to whether a token appeared", () => {
    expect(tokenWasSaved(net(), { present: false, authorized: null }, { present: true, authorized: null })).toBe(true);
    expect(tokenWasSaved(net(), null, { present: false, authorized: null })).toBe(false);
    expect(tokenWasSaved(net(), { present: true, authorized: false }, { present: true, authorized: false })).toBe(false);
  });
});
