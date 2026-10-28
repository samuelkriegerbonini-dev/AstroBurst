import {
  LOCAL_MISSING_HINT_CODE,
  LOCAL_NO_CATALOGUE_CODE,
  LOCAL_NO_SOLUTION_CODE,
  type PlateSolveResult,
  type PlateSolver,
} from "../services/astrometry";
import { formatSignificant } from "./formatSignificant";

export const ORIENTATION_TITLE =
  "How many degrees east of north the image's up direction (FITS +y) points, in astrometry.net's convention; (mirrored) marks a flipped parity. The WCS readout's rot is the header CROTA2 angle, which has the opposite sign";

export const SIP_RESIDUAL_TITLE =
  "SIPMXERR: the largest gap between the written SIP cards and the solver's distortion fit over the image. It is not the error on the sky; RMS is the fit's error at the matched stars";

const NO_VALUE = "--";
const LOCAL_SOLVER = "local";
const NOVA_SOLVER = "astrometry.net";
const DEFAULT_NOVA_HOST = "nova.astrometry.net";
const TIER0_MIN_EXPECTED_STARS = 50;
const MIN_MATCHES = 8;
const MAX_RMS_PX = 1;
const MS_PER_S = 1000;

const SOLVER_NAMES: Readonly<Record<string, string>> = {
  [LOCAL_SOLVER]: "Offline solver on this computer",
  [NOVA_SOLVER]: NOVA_SOLVER,
};

const TYCHO2_NAME = "Tycho-2";
const BUNDLED_SUFFIX = " (bundled)";

const CATALOGUE_NAMES: readonly (readonly [RegExp, string])[] = [
  [/tycho/i, TYCHO2_NAME],
  [/2mass/i, "2MASS"],
  [/siril/i, "Gaia DR3 (Siril extract)"],
  [/gaia|vizier/i, "Gaia DR3 online (VizieR)"],
];

const TIER_LABEL = /^tier(\d+)(?:-(.+))?$/;

function finite(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function catalogueName(text: string): string | null {
  if (!text) return null;
  return CATALOGUE_NAMES.find(([pattern]) => pattern.test(text))?.[1] ?? text;
}

export function formatDuration(ms: number): string {
  const total = Math.max(0, Math.floor(ms / MS_PER_S));
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
}

export function solveTierLabel(catalog: string | null | undefined, tier?: number | null): string | null {
  const label = catalog?.trim() ?? "";
  const fromLabel = TIER_LABEL.exec(label);
  const tierNumber = fromLabel ? Number(fromLabel[1]) : finite(tier);
  const name = catalogueName(fromLabel ? (fromLabel[2] ?? "") : label);
  if (tierNumber === null) return name;
  if (name === null) return `Tier ${tierNumber}`;
  return `Tier ${tierNumber}: ${name}${tierNumber === 0 && name === TYCHO2_NAME ? BUNDLED_SUFFIX : ""}`;
}

export function orientationText(orientation: number | null | undefined, parity?: string | null): string {
  const value = finite(orientation);
  if (value === null) return NO_VALUE;
  return `${value.toFixed(2)}° E of N${parity === "flipped" ? " (mirrored)" : ""}`;
}

export interface SolveField {
  key: string;
  label: string;
  value: string;
  title?: string;
}

function matchedText(matched: number | null, predicted: number | null): string | null {
  if (matched === null) return null;
  return predicted === null ? `${matched} stars` : `${matched} of ${predicted} predicted`;
}

function distortionField(result: PlateSolveResult): SolveField | null {
  if (result.sip_order === undefined && result.solver !== LOCAL_SOLVER) return null;
  const field = { key: "sip", label: "Distortion" };
  const order = finite(result.sip_order);
  if (order === null || order < 2) return { ...field, value: "TAN only (no SIP)" };
  const maxErr = finite(result.sip_max_err_px);
  if (maxErr === null) return { ...field, value: `SIP order ${order}` };
  return { ...field, value: `SIP order ${order} (SIPMXERR ${formatSignificant(maxErr, 2)} px)`, title: SIP_RESIDUAL_TITLE };
}

export function resultBlockFields(result: PlateSolveResult): SolveField[] {
  const rms = finite(result.rms_px);
  const elapsed = finite(result.elapsed_ms);
  const entries: (SolveField | null)[] = [
    result.solver ? { key: "solver", label: "Solved by", value: SOLVER_NAMES[result.solver] ?? result.solver } : null,
    ((tier) => (tier ? { key: "catalog", label: "Catalogue", value: tier } : null))(solveTierLabel(result.catalog, result.tier)),
    ((text) => (text ? { key: "matched", label: "Matched", value: text } : null))(
      matchedText(finite(result.matched), finite(result.predicted)),
    ),
    rms === null ? null : { key: "rms", label: "RMS", value: `${rms.toFixed(3)} px` },
    distortionField(result),
    { key: "orientation", label: "Orientation", value: orientationText(result.orientation, result.parity), title: ORIENTATION_TITLE },
    result.hint_source ? { key: "hint", label: "Hint", value: result.hint_source } : null,
    elapsed === null ? null : { key: "elapsed", label: "Time", value: `${(elapsed / MS_PER_S).toFixed(2)} s` },
  ];
  return entries.filter((entry): entry is SolveField => entry !== null);
}

export interface FailureCandidate {
  matched: number | null;
  predicted: number | null;
  rmsPx: number | null;
}

export type LocalFailureKind = "no_solution" | "missing_hint" | "no_catalogue" | "deadline";

export interface LocalFailure {
  kind: LocalFailureKind;
  message: string;
  best: FailureCandidate | null;
  expectedTier0Stars: number | null;
  centresTried: number | null;
  detections: number | null;
  canFallback: boolean;
  apiKeyStored: boolean | null;
  minMatches: number | null;
  maxRmsPx: number | null;
  tier0MinExpectedStars: number | null;
}

const FAILURE_KINDS: Readonly<Record<string, LocalFailureKind>> = {
  [LOCAL_NO_SOLUTION_CODE]: "no_solution",
  [LOCAL_MISSING_HINT_CODE]: "missing_hint",
  [LOCAL_NO_CATALOGUE_CODE]: "no_catalogue",
};

const FAILURE_HEADINGS: Readonly<Record<LocalFailureKind, string>> = {
  no_solution: "No offline solution",
  missing_hint: "Not enough hints to solve offline",
  no_catalogue: "No offline catalogue",
  deadline: "The offline solve ran out of time",
};

const SOLVER_LABEL_IN_TEXT = /\btier\d+(?:-[a-z0-9]+)+/gi;
const REQUEST_URL_IN_TEXT = /\burl \((https?:\/\/[^\s)]+)\)/gi;
const BARE_URL_IN_TEXT = /https?:\/\/[^\s);]+/gi;
const SENTENCE_END = /[.!?]$/;

function record(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value) ? (value as Record<string, unknown>) : null;
}

function firstNumber(source: Record<string, unknown>, ...keys: string[]): number | null {
  for (const key of keys) {
    const value = finite(source[key]);
    if (value !== null) return value;
  }
  return null;
}

function firstBoolean(source: Record<string, unknown>, ...keys: string[]): boolean | null {
  const key = keys.find((name) => typeof source[name] === "boolean");
  return key === undefined ? null : (source[key] as boolean);
}

export function localFailure(error: unknown): LocalFailure | null {
  const raised = record(error);
  const kind = raised && typeof raised.code === "string" ? FAILURE_KINDS[raised.code] : undefined;
  if (!raised || kind === undefined) return null;
  const context = record(raised.context) ?? {};
  const best = record(context.best);
  return {
    kind,
    message: typeof raised.message === "string" ? raised.message : String(raised.message ?? ""),
    best: best
      ? {
          matched: firstNumber(best, "matched"),
          predicted: firstNumber(best, "predicted"),
          rmsPx: firstNumber(best, "rms_px", "rmsPx"),
        }
      : null,
    expectedTier0Stars: firstNumber(context, "expectedTier0Stars", "expected_tier0_stars"),
    centresTried: firstNumber(context, "centresTried", "centres_tried"),
    detections: firstNumber(context, "detections"),
    canFallback: firstBoolean(context, "canFallback", "can_fallback") ?? true,
    apiKeyStored: firstBoolean(context, "apiKeyStored", "api_key_stored"),
    minMatches: firstNumber(context, "minMatches", "min_matches"),
    maxRmsPx: firstNumber(context, "maxRmsPx", "max_rms_px"),
    tier0MinExpectedStars: firstNumber(context, "tier0MinExpectedStars", "tier0_min_expected_stars"),
  };
}

export function deadlineFailure(message: string): LocalFailure {
  return {
    kind: "deadline",
    message,
    best: null,
    expectedTier0Stars: null,
    centresTried: null,
    detections: null,
    canFallback: true,
    apiKeyStored: null,
    minMatches: null,
    maxRmsPx: null,
    tier0MinExpectedStars: null,
  };
}

export function failureHeading(failure: LocalFailure): string {
  return FAILURE_HEADINGS[failure.kind];
}

function urlHost(url: string): string {
  try {
    return new URL(url).host || url;
  } catch {
    return url;
  }
}

function readableLabels(text: string): string {
  return text
    .replace(REQUEST_URL_IN_TEXT, (_, url: string) => urlHost(url))
    .replace(BARE_URL_IN_TEXT, (url) => urlHost(url))
    .replace(SOLVER_LABEL_IN_TEXT, (label) => solveTierLabel(label.toLowerCase()) ?? label);
}

function reasonLine(failure: LocalFailure): string | null {
  const heading = `${FAILURE_HEADINGS[failure.kind]}.`;
  const body = failure.message.trim().startsWith(heading) ? failure.message.trim().slice(heading.length) : failure.message;
  const text = readableLabels(body.trim());
  if (!text) return null;
  return SENTENCE_END.test(text) ? text : `${text}.`;
}

function bestLine(failure: LocalFailure): string | null {
  const best = failure.best;
  if (!best || best.matched === null) return null;
  const predicted = best.predicted === null ? "" : ` of ${best.predicted} predicted`;
  const rms = best.rmsPx === null ? "" : `, RMS ${best.rmsPx.toFixed(2)} px`;
  const minMatches = failure.minMatches ?? MIN_MATCHES;
  const maxRmsPx = failure.maxRmsPx ?? MAX_RMS_PX;
  return `Best candidate: ${best.matched}${predicted} stars matched${rms}; a solution needs ${minMatches} matches and an RMS of ${maxRmsPx} px or less.`;
}

function searchLine(centres: number | null, detections: number | null): string | null {
  if (centres === null) return null;
  const stars = detections === null ? "" : ` with ${detections} detected stars`;
  return `Searched ${centres} centre${centres === 1 ? "" : "s"}${stars}.`;
}

function tierLine(failure: LocalFailure): string | null {
  const expected = failure.expectedTier0Stars;
  const needed = failure.tier0MinExpectedStars ?? TIER0_MIN_EXPECTED_STARS;
  if (failure.kind !== "no_solution" || expected === null || expected >= needed) return null;
  return `About ${Math.round(expected)} Tycho-2 stars are expected in this field, fewer than tier 0 needs; add a deeper catalogue in Settings > Catalogs, such as a Siril Gaia DR3 extract.`;
}

export function failureLines(failure: LocalFailure): string[] {
  return [searchLine(failure.centresTried, failure.detections), bestLine(failure), reasonLine(failure), tierLine(failure)].filter(
    (line): line is string => line !== null && line !== "",
  );
}

export interface FallbackOffer {
  offered: boolean;
  note: string;
}

export const NO_API_KEY_NOTICE =
  "No astrometry.net API key is saved, so nothing was uploaded. Save it in Settings, then click Try astrometry.net again.";

const NOT_OFFERED: FallbackOffer = { offered: false, note: "" };

function hostOf(apiUrl: string | null | undefined): string {
  if (!apiUrl) return DEFAULT_NOVA_HOST;
  try {
    return new URL(apiUrl).host || DEFAULT_NOVA_HOST;
  } catch {
    return DEFAULT_NOVA_HOST;
  }
}

export function fallbackOffer(
  failure: LocalFailure | null,
  lastSolver: PlateSolver | null,
  hasApiKey: boolean,
  apiUrl?: string | null,
): FallbackOffer {
  if (!failure || !failure.canFallback || lastSolver === null || lastSolver === NOVA_SOLVER) return NOT_OFFERED;
  const host = hostOf(apiUrl);
  return hasApiKey
    ? { offered: true, note: `Uploads this image to ${host} with your API key; the solve runs there.` }
    : {
        offered: true,
        note: `Uploads this image to ${host} and needs an astrometry.net API key: save it in Settings, then click Try astrometry.net.`,
      };
}

export function solvingText(solver: PlateSolver, limit: string): string {
  if (solver === NOVA_SOLVER) return `Queued on astrometry.net; the request gives up after ${limit}.`;
  if (solver === LOCAL_SOLVER) {
    return `Solving on this computer with the offline catalogues; nothing is uploaded. Gives up after ${limit}.`;
  }
  return `Solving on this computer; the image is not uploaded. When the offline catalogues are not deep enough, the Gaia DR3 star list of the field is fetched from VizieR. Gives up after ${limit}.`;
}

export function deadlineText(solver: PlateSolver, limit: string): string {
  return solver === NOVA_SOLVER
    ? `Plate solve gave up after ${limit} (Settings > Timeout). astrometry.net may still finish the job; try again later.`
    : `The offline solve gave up after ${limit} (Settings > Timeout).`;
}
