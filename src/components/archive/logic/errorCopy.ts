import type { MastError, MastErrorCode, MastMissionId } from "../../../shared/types/mast";
import { formatBytes } from "./productView";

export type ErrorSurface = "search" | "thumb" | "preview" | "list" | "products" | "token-save" | "token-check";
export type ErrorPlacement = "blocking" | "banner" | "inline" | "item" | "roman" | "unavailable" | "silent";
export type BannerKind = "network" | "timeout" | "server" | "rate-limited" | "decode" | "cache";

export interface ErrorContextInput {
  surface: ErrorSurface;
  hasResults?: boolean;
  missionLabel?: string;
  now?: Date;
  rowPublic?: boolean;
  tokenSaved?: boolean;
}

export interface ErrorView {
  placement: ErrorPlacement;
  bannerKind?: BannerKind;
  title?: string;
  text: string;
  short: string;
  field?: "query" | "radius";
}

export const MAST_HOST_LABEL = "mast.stsci.edu";
export const DETAIL_MAX_CHARS = 300;
export const ROMAN_CONFIRMED_TEXT = "Roman access confirmed.";
export const ROMAN_NO_ACCESS_TEXT =
  "MAST accepted your token, but this account does not have Roman access yet. Access is granted by STScI (for example to programme PIs and the collaborators they add).";
export const TOKEN_EXPIRY_TEXT = "Tokens expire after 10 days without use or 60 days after creation.";
export const TOKEN_EXPIRED_TITLE = "Token on this computer: saved, no longer accepted by MAST";
export const TOKEN_REJECTED_TEXT = "MAST did not accept this token (mistyped, expired or revoked). Nothing was saved.";
export const TOKEN_CHECK_UNREACHABLE_TEXT = "MAST could not be reached to check Roman access.";
export const THUMBS_GAVE_UP_TEXT = "MAST kept limiting requests, so thumbnails stay paused.";

const DEFAULT_TIMEOUT_SEC: Record<ErrorSurface, number> = {
  search: 120,
  thumb: 30,
  preview: 120,
  list: 60,
  products: 120,
  "token-save": 30,
  "token-check": 30,
};

const DEFAULT_RETRY_AFTER_SEC = 10;
const TOKEN_MISSIONS: readonly MastMissionId[] = ["roman", "roman_cgi"];

const SHORT: Record<MastErrorCode, string> = {
  mast_network: "network",
  mast_timeout: "timed out",
  mast_cancelled: "cancelled",
  mast_rate_limited: "paused",
  mast_server: "MAST error",
  mast_rejected: "rejected",
  mast_unauthorized: "exclusive access",
  mast_no_image: "no preview",
  mast_not_deployed: "not deployed",
  mast_decode: "unexpected answer",
  mast_too_large: "too large",
  mast_not_image: "not an image",
  mast_bad_input: "invalid",
  mast_token_rejected: "token rejected",
  mast_cache: "cache",
};

const OPERATION_NAMES: [RegExp, string][] = [
  [/(^|\/)search$/, "Mission Search API"],
  [/(^|\/)get_image_list$/, "image lists"],
  [/(^|\/)get_image_thumbnail$/, "thumbnails"],
  [/(^|\/)get_image_preview$/, "previews"],
  [/(^|\/)(post_)?list_products$/, "product lists"],
];

function sentence(text: string): string {
  const trimmed = text.trim().replace(/[.\s]+$/, "");
  return trimmed ? `${trimmed}.` : "";
}

function withCause(prefix: string, cause: string): string {
  const body = cause.trim().replace(/[.\s]+$/, "");
  return body ? `${prefix}: ${body}.` : `${prefix}.`;
}

export function stripQuery(operation: string): string {
  const cut = operation.search(/[?#]/);
  return cut >= 0 ? operation.slice(0, cut) : operation;
}

function clockText(now: Date): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(now.getHours())}:${pad(now.getMinutes())}`;
}

export function operationName(operation: string | undefined): string {
  const path = stripQuery(operation ?? "");
  for (const [pattern, name] of OPERATION_NAMES) if (pattern.test(path)) return name;
  return "Mission Search API";
}

function missionName(err: MastError, ctx: ErrorContextInput): string {
  if (ctx.missionLabel) return ctx.missionLabel;
  return err.context.mission ? err.context.mission.toUpperCase() : "this mission";
}

export function rateLimitText(kind: "thumbs" | "search", seconds: number, status: number | null): string {
  const http = status ? ` (HTTP ${status})` : "";
  const action = kind === "thumbs" ? "Thumbnails are paused and resume" : "Retry";
  return `MAST is limiting requests${http}. ${action} in ${Math.max(0, Math.ceil(seconds))} s.`;
}

export function tokenSaveFailureText(tokenSaved: boolean): string {
  return tokenSaved ? "Token saved. MAST could not be reached to check Roman access." : "Could not reach MAST to check the token. Nothing was saved.";
}

function networkText(err: MastError): string {
  return withCause(`Could not reach MAST (${MAST_HOST_LABEL})`, err.message);
}

function timeoutText(err: MastError, surface: ErrorSurface): string {
  const seconds = err.context.timeoutSec ?? DEFAULT_TIMEOUT_SEC[surface];
  return `MAST did not answer within ${seconds} s. First queries of the day can take over a minute; try again, or narrow the radius or add a filter.`;
}

function serverText(err: MastError): string {
  const status = err.context.status;
  return status
    ? `MAST had a problem answering (HTTP ${status}). This is on MAST's side; try again in a moment.`
    : "MAST had a problem answering. This is on MAST's side; try again in a moment.";
}

const DECODE_TEXT = "MAST answered in a format AstroBurst does not understand. AstroBurst may need an update.";
const NOT_IMAGE_TEXT = "MAST returned something that is not an image.";

function tooLargeText(err: MastError): string {
  const bytes = err.context.bytes;
  const size = bytes !== undefined && Number.isFinite(bytes) && bytes > 0 ? ` (${formatBytes(bytes)})` : "";
  return `This image is larger than AstroBurst previews${size}. It will be available through downloads.`;
}

function noImageText(err: MastError, rowPublic: boolean): string {
  if (!rowPublic) return "No preview for this image (exclusive access).";
  const detail = err.context.detail?.trim().replace(/[.\s]+$/, "");
  if (detail) return `No preview for this image (MAST: ${detail.slice(0, DETAIL_MAX_CHARS)}).`;
  if (err.context.status) return `No preview for this image (MAST: HTTP ${err.context.status}).`;
  return "No preview for this image.";
}

function rejectedText(err: MastError, surface: ErrorSurface): string {
  const detail = (err.context.detail ?? err.message).slice(0, DETAIL_MAX_CHARS);
  const what = surface === "search" ? "the query" : "the request";
  return withCause(`MAST rejected ${what}`, detail);
}

function cacheText(err: MastError): string {
  const cause = err.message.trim().replace(/[.\s]+$/, "");
  return cause
    ? `AstroBurst could not write its MAST cache (${cause}). Previews still load but are not kept.`
    : "AstroBurst could not write its MAST cache. Previews still load but are not kept.";
}

function notDeployedText(err: MastError, ctx: ErrorContextInput): string {
  const when = clockText(ctx.now ?? new Date());
  return `MAST does not serve the ${missionName(err, ctx)} ${operationName(err.context.operation)} right now (checked ${when}).`;
}

function inlineField(text: string): "query" | "radius" {
  return /radius/i.test(text) ? "radius" : "query";
}

const BLOCKING_TITLES: Partial<Record<BannerKind, string>> = {
  network: `Could not reach MAST (${MAST_HOST_LABEL})`,
  timeout: "MAST did not answer in time",
  server: "MAST had a problem answering",
  decode: "MAST answered in an unexpected format",
};

function searchLevel(kind: BannerKind, text: string, err: MastError, ctx: ErrorContextInput): ErrorView {
  const short = SHORT[err.code];
  if (ctx.hasResults) return { placement: "banner", bannerKind: kind, text, short };
  const blockingText = kind === "network" ? `The connection failed: ${sentence(err.message) || "no answer."} Your query is kept.` : text;
  return { placement: "blocking", bannerKind: kind, title: BLOCKING_TITLES[kind], text: blockingText, short };
}

function itemLevel(kind: BannerKind, text: string, err: MastError): ErrorView {
  return { placement: "item", bannerKind: kind, text, short: SHORT[err.code] };
}

function tokenView(err: MastError, ctx: ErrorContextInput): ErrorView {
  const short = SHORT[err.code];
  if (err.code === "mast_cancelled") return { placement: "silent", text: "", short };
  if (err.code === "mast_token_rejected") {
    return ctx.surface === "token-save"
      ? { placement: "roman", text: TOKEN_REJECTED_TEXT, short }
      : { placement: "roman", title: TOKEN_EXPIRED_TITLE, text: TOKEN_EXPIRY_TEXT, short };
  }
  if (err.code === "mast_unauthorized") return { placement: "roman", text: ROMAN_NO_ACCESS_TEXT, short };
  if (err.code === "mast_bad_input") return { placement: "roman", text: err.message, short };
  const text = ctx.surface === "token-save" ? tokenSaveFailureText(ctx.tokenSaved ?? false) : TOKEN_CHECK_UNREACHABLE_TEXT;
  return { placement: "roman", text, short };
}

export function errorView(err: MastError, ctx: ErrorContextInput): ErrorView {
  const { surface } = ctx;
  if (surface === "token-save" || surface === "token-check") return tokenView(err, ctx);
  const short = SHORT[err.code];
  const isSearch = surface === "search";
  const rowPublic = ctx.rowPublic ?? true;
  const level = (kind: BannerKind, text: string) => (isSearch ? searchLevel(kind, text, err, ctx) : itemLevel(kind, text, err));
  switch (err.code) {
    case "mast_cancelled":
      return { placement: "silent", text: "", short };
    case "mast_network":
      return level("network", networkText(err));
    case "mast_timeout":
      return level("timeout", timeoutText(err, surface));
    case "mast_server":
      return level("server", serverText(err));
    case "mast_decode":
      return level("decode", DECODE_TEXT);
    case "mast_cache":
      return { placement: "banner", bannerKind: "cache", text: cacheText(err), short };
    case "mast_rate_limited": {
      const seconds = err.context.retryAfterSec ?? DEFAULT_RETRY_AFTER_SEC;
      const status = err.context.status ?? null;
      if (surface === "thumb") return { placement: "banner", bannerKind: "rate-limited", text: rateLimitText("thumbs", seconds, status), short };
      const text = rateLimitText("search", seconds, status);
      return isSearch ? { placement: "banner", bannerKind: "rate-limited", text, short } : { placement: "item", bannerKind: "rate-limited", text, short };
    }
    case "mast_rejected": {
      const text = rejectedText(err, surface);
      return isSearch ? { placement: "inline", text, short, field: inlineField(text) } : { placement: "item", text, short };
    }
    case "mast_bad_input":
      return isSearch ? { placement: "inline", text: err.message, short, field: inlineField(err.message) } : { placement: "item", text: err.message, short };
    case "mast_unauthorized":
      if (isSearch || (err.context.mission && TOKEN_MISSIONS.includes(err.context.mission))) return { placement: "roman", text: ROMAN_NO_ACCESS_TEXT, short: "token needed" };
      return { placement: "item", text: noImageText(err, false), short: "exclusive access" };
    case "mast_token_rejected":
      return { placement: "roman", title: TOKEN_EXPIRED_TITLE, text: TOKEN_EXPIRY_TEXT, short };
    case "mast_no_image":
      if (isSearch) return level("server", noImageText(err, rowPublic));
      return { placement: "item", text: noImageText(err, rowPublic), short: rowPublic ? short : "exclusive access" };
    case "mast_not_deployed": {
      const text = notDeployedText(err, ctx);
      return isSearch ? { placement: "unavailable", text, short } : { placement: "item", text, short };
    }
    case "mast_too_large":
      return isSearch ? level("server", tooLargeText(err)) : { placement: "item", text: tooLargeText(err), short };
    case "mast_not_image":
      return isSearch ? level("decode", NOT_IMAGE_TEXT) : { placement: "item", text: NOT_IMAGE_TEXT, short };
  }
}

export function errorDetails(err: MastError): string[] {
  const lines = [`Code: ${err.code}`];
  if (err.context.status !== undefined) lines.push(`HTTP status: ${err.context.status}`);
  if (err.context.operation) lines.push(`Operation: ${stripQuery(err.context.operation)}`);
  if (err.context.detail) lines.push(`MAST detail: ${err.context.detail.slice(0, DETAIL_MAX_CHARS)}`);
  return lines;
}
