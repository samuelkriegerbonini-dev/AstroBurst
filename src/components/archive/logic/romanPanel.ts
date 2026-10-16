import type { MastError, MastTokenStatus } from "../../../shared/types/mast";
import { ROMAN_CONFIRMED_TEXT, ROMAN_NO_ACCESS_TEXT, TOKEN_EXPIRY_TEXT, TOKEN_REJECTED_TEXT } from "./errorCopy";
import { clockText } from "./missionState";

export type RomanPanelState = "none" | "checking" | "authorized" | "not-authorized" | "rejected" | "expired" | "unreachable";
export type RomanAction = "open-page" | "check" | "replace" | "remove" | "cancel" | "try-again" | "manage" | "collapse" | "cancel-replace";
export type RomanTone = "ok" | "warn" | "error";

export interface RomanPanelInput {
  state: RomanPanelState;
  present: boolean;
  message: string | null;
  checkedAt: number | null;
  replacing: boolean;
  expanded: boolean;
  checkedOn: string;
}

export interface RomanPanelView {
  layout: "compact" | "full";
  title: string;
  intro: string | null;
  needs: string[] | null;
  tokenLine: string | null;
  message: string | null;
  tone: RomanTone | null;
  checking: string[] | null;
  form: boolean;
  actions: RomanAction[];
}

export const ROMAN_PANEL_TITLE = "Roman access";
export const ROMAN_NEEDS_TITLE = "Two things are needed:";
export const ROMAN_NEEDS = [
  "A MAST API token, created on auth.mast.stsci.edu after signing in with your MyST account.",
  "Roman access on that account, granted by STScI. A token alone is not enough.",
];
export const TOKEN_PAGE_NOTE = "opens auth.mast.stsci.edu/token in your browser";
export const TOKEN_STORAGE_NOTE =
  "The token is stored only on this computer, sent only to mast.stsci.edu for Roman requests and to auth.mast.stsci.edu to check it, and never shown again.";
export const TOKEN_FIELD_LABEL = "MAST token";
export const TOKEN_SUBMIT_LABEL = "Check and save";
export const CHECKING_TITLE = "Checking with MAST";
export const CHECK_STEPS = ["Token accepted by auth.mast.stsci.edu", "Roman access"];
export const NOT_CHECKED_TEXT = "AstroBurst has not checked Roman access with MAST in this session.";
export const OPEN_PAGE_FAILED_TEXT = "Could not open the browser. The token page is auth.mast.stsci.edu/token.";
export const TOKEN_REMOVED_TEXT = "Token removed from this computer.";
export const ROMAN_SEARCH_HINT = "Search disabled: Roman needs an authorized MAST token";

export const ACTION_LABELS: Record<RomanAction, string> = {
  "open-page": "Open the MAST token page",
  check: "Check again",
  replace: "Replace token",
  remove: "Remove token",
  cancel: "Cancel",
  "try-again": "Try again",
  manage: "Manage token",
  collapse: "Done",
  "cancel-replace": "Keep the saved token",
};

const TOKEN_LINE = "Token on this computer: ";
const MANAGE: RomanAction[] = ["check", "replace", "remove"];

export function romanIntro(checkedOn: string): string {
  return `MAST answers Roman searches only for authorized users. Without a token MAST replies "Only authorized users can make search API calls" (HTTP 401, checked ${checkedOn}).`;
}

function checked(at: number | null): string {
  return at === null ? "" : ` (checked ${clockText(at)})`;
}

function tokenLine(input: RomanPanelInput): string | null {
  const { state, present, checkedAt } = input;
  switch (state) {
    case "checking":
      return null;
    case "authorized":
      return `${TOKEN_LINE}saved, Roman access confirmed${checked(checkedAt)}`;
    case "not-authorized":
      return `${TOKEN_LINE}saved, accepted by MAST, no Roman access${checked(checkedAt)}`;
    case "expired":
      return `${TOKEN_LINE}saved, no longer accepted by MAST`;
    case "rejected":
      return `${TOKEN_LINE}${present ? "saved, unchanged" : "none"}`;
    case "unreachable":
      return `${TOKEN_LINE}${present ? "saved, Roman access not checked" : "none"}`;
    case "none":
      return `${TOKEN_LINE}none`;
  }
}

function outcome(input: RomanPanelInput): { message: string | null; tone: RomanTone | null } {
  switch (input.state) {
    case "authorized":
      return { message: ROMAN_CONFIRMED_TEXT, tone: "ok" };
    case "not-authorized":
      return { message: ROMAN_NO_ACCESS_TEXT, tone: "warn" };
    case "rejected":
      return { message: input.message ?? TOKEN_REJECTED_TEXT, tone: "error" };
    case "expired":
      return { message: TOKEN_EXPIRY_TEXT, tone: "warn" };
    case "unreachable":
      if (!input.present) return { message: input.message, tone: "error" };
      return { message: input.message ?? NOT_CHECKED_TEXT, tone: "warn" };
    default:
      return { message: null, tone: null };
  }
}

function baseActions(input: RomanPanelInput): RomanAction[] {
  switch (input.state) {
    case "none":
      return ["open-page"];
    case "checking":
      return ["cancel"];
    case "authorized":
      return input.expanded ? [...MANAGE, "collapse"] : ["manage"];
    case "not-authorized":
      return MANAGE;
    case "rejected":
      return input.present ? ["open-page", "remove"] : ["open-page"];
    case "expired":
      return ["open-page", "replace", "remove"];
    case "unreachable":
      return input.present ? MANAGE : ["try-again", "open-page"];
  }
}

export function romanPanelView(input: RomanPanelInput): RomanPanelView {
  const { state, present } = input;
  const checking = state === "checking";
  const replacing = input.replacing && present && !checking;
  const formState = state === "none" || state === "rejected" || (state === "unreachable" && !present);
  const compact = state === "authorized" && !input.expanded && !replacing;
  const actions = replacing ? [...baseActions(input).filter((a) => a !== "replace" && a !== "collapse"), "cancel-replace" as const] : baseActions(input);
  const { message, tone } = outcome(input);
  return {
    layout: compact ? "compact" : "full",
    title: ROMAN_PANEL_TITLE,
    intro: state === "authorized" ? null : romanIntro(input.checkedOn),
    needs: present || state === "authorized" ? null : ROMAN_NEEDS,
    tokenLine: tokenLine(input),
    message,
    tone,
    checking: checking ? CHECK_STEPS : null,
    form: !checking && (formState || replacing),
    actions,
  };
}

const AUTH_CHECK_OPERATION = /(^|\/)auth_check$/;
const INFO_OPERATION = /(^|\/)info$/;

export function tokenWasSaved(error: MastError, before: MastTokenStatus | null, after: MastTokenStatus): boolean {
  const operation = error.context.operation ?? "";
  if (AUTH_CHECK_OPERATION.test(operation)) return true;
  if (INFO_OPERATION.test(operation)) return false;
  return !(before?.present ?? false) && after.present;
}
