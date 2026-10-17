import type { MastError, MastErrorCode, MastMissionId } from "../../../shared/types/mast";
import { missionLabelOf } from "./missionLabels";
import { formatBytes } from "./productView";
import { elapsedText } from "./resultsView";

export type ErrorSurface = "search" | "thumb" | "preview" | "list" | "products" | "token-save" | "token-check";
export type ErrorPlacement = "blocking" | "banner" | "inline" | "item" | "roman" | "unavailable" | "silent";
export type BannerKind = "network" | "timeout" | "server" | "rate-limited" | "decode" | "cache";
export type SlotSurface = "thumb" | "preview" | "list";
export type SlotTone = "server" | "transient" | "muted";

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

export interface SlotErrorView {
  code: MastErrorCode;
  tone: SlotTone;
  title: string;
  meta: string | null;
  detail: string | null;
  badge: string;
  retry: boolean;
  summary: string;
  facts: string | null;
  lines: string[];
}

export interface SlotContext {
  surface: SlotSurface;
  rowPublic?: boolean;
  waitedMs?: number | null;
}

export interface PausedQueue {
  pausedUntil: number | null;
  status: number | null;
  gaveUp: boolean;
}

export const DETAIL_MAX_CHARS = 300;
export const MISSING_FILE: MastError = { code: "mast_cache", message: "the cached image file is missing", context: {} };
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
  mast_size_mismatch: "size mismatch",
  mast_busy: "busy",
  mast_stale: "newer at MAST",
  mast_quota: "quota",
  mast_uri_refused: "not downloadable",
  mast_peek_incomplete: "header not reached",
};

const OPERATION_NAMES: [RegExp, string][] = [
  [/(^|\/)search$/, "Mission Search API"],
  [/(^|\/)get_image_list$/, "image lists"],
  [/(^|\/)get_image_thumbnail$/, "thumbnails"],
  [/(^|\/)get_image_preview$/, "previews"],
  [/(^|\/)(post_)?list_products$/, "product lists"],
];

const TIMEOUT_SUBJECTS: [RegExp, string][] = [
  [/(^|\/)get_image_list$/, "the image list"],
  [/(^|\/)get_image_thumbnail$/, "this thumbnail"],
  [/(^|\/)get_image_preview$/, "this preview"],
  [/(^|\/)(post_)?list_products$/, "the product list"],
];

const SURFACE_SUBJECT: Partial<Record<ErrorSurface, string>> = {
  thumb: "this thumbnail",
  preview: "this preview",
  list: "the image list",
  products: "the product list",
};

const SURFACE_OPERATION: Record<SlotSurface, string> = { thumb: "thumbnails", preview: "previews", list: "image lists" };

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
  return err.context.mission ? missionLabelOf(err.context.mission) : "this mission";
}

export function rateLimitText(kind: "thumbs" | "search", seconds: number, status: number | null): string {
  const http = status ? ` (HTTP ${status})` : "";
  const action = kind === "thumbs" ? "Thumbnails are paused and resume" : "Retry";
  return `MAST is limiting requests${http}. ${action} in ${Math.max(0, Math.ceil(seconds))} s.`;
}

export function tokenSaveFailureText(tokenSaved: boolean): string {
  return tokenSaved ? "Token saved. MAST could not be reached to check Roman access." : "Could not reach MAST to check the token. Nothing was saved.";
}

function unreachableTitle(err: MastError): string {
  const host = err.context.host?.trim();
  return host ? `Could not reach MAST (${host})` : "Could not reach MAST";
}

function networkText(err: MastError): string {
  return withCause(unreachableTitle(err), err.message);
}

function timeoutSubject(err: MastError, surface: ErrorSurface): string | null {
  if (surface === "search") return null;
  const path = stripQuery(err.context.operation ?? "");
  for (const [pattern, subject] of TIMEOUT_SUBJECTS) if (pattern.test(path)) return subject;
  return SURFACE_SUBJECT[surface] ?? null;
}

function triesText(err: MastError): string {
  const attempts = err.context.attempts;
  return attempts !== undefined && attempts > 1 ? ` (${attempts} tries)` : "";
}

function timeoutText(err: MastError, surface: ErrorSurface): string {
  const seconds = err.context.timeoutSec ?? DEFAULT_TIMEOUT_SEC[surface];
  const subject = timeoutSubject(err, surface);
  if (subject) return `MAST did not send ${subject} within ${seconds} s${triesText(err)}. Try again in a moment.`;
  return `MAST did not answer within ${seconds} s. First queries of the day can take over a minute; try again, or narrow the radius or add a filter.`;
}

function trimmedDetail(err: MastError): string | null {
  const detail = err.context.detail?.trim().replace(/[.\s]+$/, "");
  return detail ? detail.slice(0, DETAIL_MAX_CHARS) : null;
}

function serverText(err: MastError): string {
  const status = err.context.status;
  const detail = trimmedDetail(err);
  const said = status && detail ? ` (HTTP ${status}: ${detail})` : status ? ` (HTTP ${status})` : detail ? ` (${detail})` : "";
  return `MAST had a problem answering${said}. This is on MAST's side; try again in a moment.`;
}

const DECODE_TEXT = "MAST answered in a format AstroBurst does not understand. AstroBurst may need an update.";
const EMPTY_ANSWER = "empty answer";

function isLocalFault(err: MastError): boolean {
  return err.code === "mast_decode" && !err.context.operation;
}

function decodeText(err: MastError): string {
  if (isLocalFault(err)) return `${withCause("AstroBurst hit an internal error", err.message)} This is not a MAST problem; try again, and report it if it keeps happening.`;
  if (err.message.trim().toLowerCase() === EMPTY_ANSWER) return `MAST sent an empty answer for the ${operationName(err.context.operation)}. Try again in a moment.`;
  return DECODE_TEXT;
}
const NOT_IMAGE_TEXT = "MAST returned something that is not an image.";

function tooLargeText(err: MastError): string {
  const bytes = err.context.bytes;
  const size = bytes !== undefined && Number.isFinite(bytes) && bytes > 0 ? ` (${formatBytes(bytes)})` : "";
  return `This image is larger than AstroBurst previews${size}. It will be available through downloads.`;
}

function noImageText(err: MastError, rowPublic: boolean): string {
  if (!rowPublic) return "No preview for this image (exclusive access).";
  const detail = trimmedDetail(err);
  if (detail) return `No preview for this image (MAST: ${detail}).`;
  if (err.context.status) return `No preview for this image (MAST: HTTP ${err.context.status}).`;
  return "No preview for this image.";
}

function rejectedText(err: MastError, surface: ErrorSurface): string {
  const detail = (err.context.detail ?? err.message).slice(0, DETAIL_MAX_CHARS);
  const what = surface === "search" ? "the query" : "the request";
  return withCause(`MAST rejected ${what}`, detail);
}

const CACHE_TROUBLE_HINT = "Images from MAST cannot be shown until it works again: free disk space or run Clear MAST cache, then retry.";
export const CACHE_TROUBLE_TEXT = `AstroBurst could not use its MAST cache. ${CACHE_TROUBLE_HINT}`;

function cacheText(err: MastError): string {
  const cause = err.message.trim().replace(/[.\s]+$/, "");
  return cause ? `AstroBurst could not use its MAST cache (${cause}). ${CACHE_TROUBLE_HINT}` : CACHE_TROUBLE_TEXT;
}

function notDeployedText(err: MastError, ctx: ErrorContextInput): string {
  const when = clockText(ctx.now ?? new Date());
  return `MAST does not serve the ${missionName(err, ctx)} ${operationName(err.context.operation)} right now (checked ${when}).`;
}

function inlineField(text: string): "query" | "radius" {
  return /radius/i.test(text) ? "radius" : "query";
}

const BLOCKING_TITLES: Partial<Record<BannerKind, string>> = {
  timeout: "MAST did not answer in time",
  server: "MAST had a problem answering",
  decode: "MAST answered in an unexpected format",
};

function blockingTitle(kind: BannerKind, err: MastError): string | undefined {
  if (kind === "network") return unreachableTitle(err);
  if (isLocalFault(err)) return "AstroBurst error";
  return BLOCKING_TITLES[kind];
}

function searchLevel(kind: BannerKind, text: string, err: MastError, ctx: ErrorContextInput): ErrorView {
  const short = SHORT[err.code];
  if (ctx.hasResults) return { placement: "banner", bannerKind: kind, text, short };
  const blockingText = kind === "network" ? `The connection failed: ${sentence(err.message) || "no answer."} Your query is kept.` : text;
  return { placement: "blocking", bannerKind: kind, title: blockingTitle(kind, err), text: blockingText, short };
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
      return level("decode", decodeText(err));
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
    case "mast_size_mismatch":
    case "mast_busy":
    case "mast_stale":
    case "mast_quota":
    case "mast_uri_refused":
    case "mast_peek_incomplete":
      return { placement: "item", text: sentence(err.message), short };
  }
}

export function errorDetails(err: MastError): string[] {
  const c = err.context;
  const lines = [`Code: ${err.code}`];
  if (c.status !== undefined) lines.push(`HTTP status: ${c.status}`);
  if (c.operation) lines.push(`Operation: ${stripQuery(c.operation)}`);
  if (c.host) lines.push(`Host: ${c.host}`);
  if (c.detail) lines.push(`MAST detail: ${c.detail.slice(0, DETAIL_MAX_CHARS)}`);
  if (c.attempts !== undefined) lines.push(`Attempts: ${c.attempts}`);
  if (c.elapsedMs !== undefined) lines.push(`Time at MAST: ${elapsedText(c.elapsedMs)}`);
  return lines;
}

interface SlotCopy {
  tone: SlotTone;
  title: string;
  meta: string | null;
  detail?: string | null;
  badge: string;
  retry: boolean;
}

function joined(parts: (string | null | undefined)[], sep = " · "): string | null {
  const kept = parts.map((p) => p?.trim()).filter((p): p is string => !!p);
  return kept.length ? kept.join(sep) : null;
}

function httpText(status: number | undefined): string | null {
  return status !== undefined ? `HTTP ${status}` : null;
}

function slotOperation(err: MastError, surface: SlotSurface): string {
  return err.context.operation ? operationName(err.context.operation) : SURFACE_OPERATION[surface];
}

function causeText(err: MastError): string | null {
  return err.message.trim().replace(/[.\s]+$/, "") || null;
}

function isMissingFile(err: MastError): boolean {
  return err.code === MISSING_FILE.code && err.message === MISSING_FILE.message;
}

function slotCopy(err: MastError, ctx: SlotContext): SlotCopy | null {
  const c = err.context;
  const status = c.status;
  const waited = ctx.waitedMs !== undefined && ctx.waitedMs !== null ? `waited ${elapsedText(ctx.waitedMs)}` : null;
  const exclusive: SlotCopy = { tone: "muted", title: "Exclusive access", meta: httpText(status), badge: "EXCL", retry: false };
  switch (err.code) {
    case "mast_cancelled":
      return null;
    case "mast_server":
      return { tone: "server", title: "MAST server error", meta: httpText(status), detail: trimmedDetail(err), badge: status ? String(status) : "SRV", retry: true };
    case "mast_timeout": {
      const seconds = c.timeoutSec ?? DEFAULT_TIMEOUT_SEC[ctx.surface];
      const tries = c.attempts !== undefined && c.attempts > 1 ? `${c.attempts} tries` : null;
      return { tone: "transient", title: "MAST did not answer", meta: joined([`no answer within ${seconds} s`, tries, waited]), badge: `${seconds}s`, retry: true };
    }
    case "mast_network":
      return { tone: "transient", title: "Could not reach MAST", meta: joined([c.host, causeText(err) ?? "no answer"]), badge: "NET", retry: true };
    case "mast_no_image":
      if (ctx.rowPublic === false) return exclusive;
      return {
        tone: "muted",
        title: "No image at MAST",
        meta: status !== undefined ? joined([httpText(status), trimmedDetail(err)]) : (trimmedDetail(err) ?? causeText(err)),
        badge: status !== undefined ? String(status) : "none",
        retry: true,
      };
    case "mast_unauthorized":
      return exclusive;
    case "mast_rejected":
      return { tone: "server", title: "MAST refused the request", meta: httpText(status), detail: trimmedDetail(err) ?? causeText(err), badge: status ? String(status) : "REJ", retry: true };
    case "mast_decode":
      if (isLocalFault(err)) return { tone: "transient", title: "AstroBurst error", meta: causeText(err), badge: "ERR", retry: true };
      return { tone: "server", title: "Unexpected answer from MAST", meta: joined([slotOperation(err, ctx.surface), causeText(err)]), badge: "FMT", retry: true };
    case "mast_not_image":
      return { tone: "server", title: "MAST sent something that is not an image", meta: slotOperation(err, ctx.surface), badge: "IMG", retry: true };
    case "mast_too_large":
      return { tone: "muted", title: "Too large to preview", meta: c.bytes !== undefined && Number.isFinite(c.bytes) && c.bytes > 0 ? formatBytes(c.bytes) : null, badge: "BIG", retry: false };
    case "mast_rate_limited":
      return { tone: "transient", title: "Paused by MAST", meta: joined([httpText(status), c.retryAfterSec !== undefined ? `resumes in ${c.retryAfterSec} s` : null]), badge: "WAIT", retry: false };
    case "mast_cache":
      if (isMissingFile(err)) return { tone: "transient", title: "Cached file missing", meta: null, badge: "DISK", retry: true };
      return { tone: "transient", title: "AstroBurst cache error", meta: causeText(err), badge: "DISK", retry: true };
    case "mast_not_deployed":
      return { tone: "muted", title: "Not served by MAST", meta: joined([c.mission ? missionLabelOf(c.mission) : null, slotOperation(err, ctx.surface)], " "), badge: "N/A", retry: false };
    case "mast_bad_input":
      return { tone: "muted", title: "Request not sent", meta: causeText(err), badge: "BAD", retry: false };
    case "mast_token_rejected":
      return { tone: "muted", title: "Token not accepted", meta: null, badge: "KEY", retry: false };
    case "mast_size_mismatch":
      return { tone: "server", title: "Size differs from the listing", meta: causeText(err), badge: "SIZE", retry: true };
    case "mast_busy":
      return { tone: "transient", title: "Already downloading", meta: causeText(err), badge: "BUSY", retry: false };
    case "mast_stale":
      return { tone: "muted", title: "Newer version at MAST", meta: causeText(err), badge: "NEW", retry: false };
    case "mast_quota":
      return { tone: "muted", title: "Download quota reached", meta: causeText(err), badge: "QUOTA", retry: false };
    case "mast_uri_refused":
      return { tone: "muted", title: "Not downloadable", meta: causeText(err), badge: "URI", retry: false };
    case "mast_peek_incomplete":
      return { tone: "muted", title: "Header beyond the peek window", meta: causeText(err), badge: "PEEK", retry: false };
  }
}

function slotLines(err: MastError, ctx: SlotContext): string[] {
  const lines = errorDetails(err);
  if (err.code === "mast_timeout") lines.push(`Timeout: ${err.context.timeoutSec ?? DEFAULT_TIMEOUT_SEC[ctx.surface]} s per request (${slotOperation(err, ctx.surface)})`);
  if (ctx.waitedMs !== undefined && ctx.waitedMs !== null) lines.push(`Waited: ${elapsedText(ctx.waitedMs)}`);
  return lines;
}

function slotFacts(err: MastError, ctx: SlotContext): string | null {
  const c = err.context;
  const lead = c.status !== undefined ? httpText(c.status) : err.code === "mast_timeout" ? `no answer within ${c.timeoutSec ?? DEFAULT_TIMEOUT_SEC[ctx.surface]} s` : err.code === "mast_network" ? (c.host ?? null) : null;
  const tries = c.attempts !== undefined && c.attempts > 1 ? `${c.attempts} tries` : null;
  const waited = ctx.waitedMs !== undefined && ctx.waitedMs !== null ? `waited ${elapsedText(ctx.waitedMs)}` : null;
  return joined([lead, slotOperation(err, ctx.surface), tries, waited]);
}

function slotView(code: MastErrorCode, copy: SlotCopy, lines: string[], facts: string | null): SlotErrorView {
  const detail = copy.detail ?? null;
  return { code, tone: copy.tone, title: copy.title, meta: copy.meta, detail, badge: copy.badge, retry: copy.retry, summary: joined([copy.title, copy.meta, detail], ", ") ?? copy.title, facts, lines };
}

export function slotError(err: MastError, ctx: SlotContext): SlotErrorView | null {
  const copy = slotCopy(err, ctx);
  return copy ? slotView(err.code, copy, slotLines(err, ctx), slotFacts(err, ctx)) : null;
}

export const RETRY_HINT = "Press R to retry";

export function slotCellTitle(view: SlotErrorView): string {
  return slotTooltip(view, RETRY_HINT);
}

export function slotTooltip(view: SlotErrorView, hint?: string): string {
  return [view.title, view.meta, view.detail, ...view.lines, view.retry ? hint : null].filter(Boolean).join("\n");
}

export function pausedSlot(q: PausedQueue, now: number): SlotErrorView {
  const http = httpText(q.status ?? undefined);
  const left = q.pausedUntil !== null ? Math.max(0, Math.ceil((q.pausedUntil - now) / 1000)) : 0;
  const when = q.gaveUp ? "stopped, press R to resume" : left > 0 ? `resumes in ${left} s` : "resuming";
  const lines = ["Code: mast_rate_limited", ...(http ? [`HTTP status: ${q.status}`] : []), q.gaveUp ? THUMBS_GAVE_UP_TEXT : rateLimitText("thumbs", left, q.status)];
  const meta = joined([http, when]);
  return slotView("mast_rate_limited", { tone: "transient", title: "Paused by MAST", meta, badge: "WAIT", retry: false }, lines, meta);
}
