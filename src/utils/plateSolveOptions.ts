import type { CatalogDatasetReport } from "../services/catalogStore";
import type { MaxSipOrder, PlateSolver } from "../shared/types/astrometry";
import { SIRIL_GAIA_ID } from "./catalogRows";
import { decimalFieldState, type DecimalFieldOptions } from "./decimalText";
import { MAX_OFFLINE_NEAR_RADIUS_DEG, offlineNearRadius, type HintField, type HintTexts, type ScaleUnits } from "./plateSolveScale";

export type HintPolicy = "header" | "typed";
export type SipOrderChoice = "auto" | MaxSipOrder;

export interface SolveOptions {
  onlineGaia: boolean;
  offerNova: boolean;
  useTwomass: boolean;
  maxSipOrder: SipOrderChoice;
  searchRadius: string;
  hints: HintPolicy;
}

export interface StoredSolveOptions {
  options: SolveOptions;
  typedHints: Partial<HintTexts>;
}

export type SolveOptionsStorage = Pick<Storage, "getItem" | "setItem">;

export const SOLVE_OPTIONS_KEY = "astroburst.plateSolve.options.v1";
export const DEFAULT_SOLVE_OPTIONS: SolveOptions = {
  onlineGaia: true,
  offerNova: true,
  useTwomass: true,
  maxSipOrder: "auto",
  searchRadius: "",
  hints: "header",
};
export const DEFAULT_SEARCH_RADIUS_TEXT = "3";
export const SEARCH_RADIUS_FIELD: DecimalFieldOptions = { optional: true, min: 0, max: MAX_OFFLINE_NEAR_RADIUS_DEG };
export const SIP_ORDER_CHOICES: readonly SipOrderChoice[] = ["auto", 2, 3, 4];

const NO_TYPED_HINTS: Partial<HintTexts> = {};
const TEXT_HINT_FIELDS: readonly HintField[] = ["scaleLow", "scaleHigh", "centerRa", "centerDec", "radius"];
const SCALE_UNITS: readonly ScaleUnits[] = ["arcsecperpix", "arcminwidth", "degwidth"];
const TWOMASS_NAME = "2MASS";
const OFFLINE_CATALOGUES: readonly (readonly [string, string])[] = [
  ["tycho2-l6-v1", "Tycho-2"],
  [SIRIL_GAIA_ID, "Gaia DR3 (Siril extract)"],
  ["2mass-l8-v1", TWOMASS_NAME],
];

export function solveOptionsStorage(): SolveOptionsStorage | null {
  try {
    return typeof window !== "undefined" ? (window.localStorage ?? null) : null;
  } catch {
    return null;
  }
}

function storedFlag(value: unknown, fallback: boolean): boolean {
  return typeof value === "boolean" ? value : fallback;
}

function storedRadius(value: unknown): string {
  return typeof value === "string" && decimalFieldState(value, SEARCH_RADIUS_FIELD).error === null ? value : DEFAULT_SOLVE_OPTIONS.searchRadius;
}

export function sipOrderChoice(value: unknown): SipOrderChoice {
  return SIP_ORDER_CHOICES.find((choice) => choice === value || String(choice) === value) ?? DEFAULT_SOLVE_OPTIONS.maxSipOrder;
}

function storedTypedHints(value: unknown): Partial<HintTexts> {
  if (typeof value !== "object" || value === null) return NO_TYPED_HINTS;
  const record = value as Record<string, unknown>;
  const typed: Partial<HintTexts> = {};
  for (const field of TEXT_HINT_FIELDS) {
    if (typeof record[field] === "string") Object.assign(typed, { [field]: record[field] });
  }
  const units = SCALE_UNITS.find((unit) => unit === record.scaleUnits);
  if (units) typed.scaleUnits = units;
  return typed;
}

export function loadSolveOptions(storage: SolveOptionsStorage | null): StoredSolveOptions {
  const defaults: StoredSolveOptions = { options: DEFAULT_SOLVE_OPTIONS, typedHints: NO_TYPED_HINTS };
  if (!storage) return defaults;
  try {
    const text = storage.getItem(SOLVE_OPTIONS_KEY);
    if (!text) return defaults;
    const raw = JSON.parse(text) as Record<string, unknown>;
    const hints: HintPolicy = raw.hints === "typed" ? "typed" : "header";
    return {
      options: {
        onlineGaia: storedFlag(raw.onlineGaia, DEFAULT_SOLVE_OPTIONS.onlineGaia),
        offerNova: storedFlag(raw.offerNova, DEFAULT_SOLVE_OPTIONS.offerNova),
        useTwomass: storedFlag(raw.useTwomass, DEFAULT_SOLVE_OPTIONS.useTwomass),
        maxSipOrder: sipOrderChoice(raw.maxSipOrder),
        searchRadius: storedRadius(raw.searchRadius),
        hints,
      },
      typedHints: hints === "typed" ? storedTypedHints(raw.typedHints) : NO_TYPED_HINTS,
    };
  } catch {
    return defaults;
  }
}

export function saveSolveOptions(storage: SolveOptionsStorage | null, options: SolveOptions, typedHints: Partial<HintTexts>): void {
  if (!storage) return;
  try {
    storage.setItem(SOLVE_OPTIONS_KEY, JSON.stringify({ ...options, typedHints }));
  } catch {
  }
}

export function optionsSolver(options: Pick<SolveOptions, "onlineGaia">): PlateSolver {
  return options.onlineGaia ? "auto" : "local";
}

export function searchRadiusError(options: Pick<SolveOptions, "searchRadius">): string | null {
  return decimalFieldState(options.searchRadius, SEARCH_RADIUS_FIELD).error;
}

export function offlineSearchRadius(hintRadius: number | null, options: Pick<SolveOptions, "searchRadius">): number | undefined {
  if (hintRadius !== null) return offlineNearRadius(hintRadius);
  return decimalFieldState(options.searchRadius, SEARCH_RADIUS_FIELD).value ?? undefined;
}

export function searchRadiusPlaceholder(options: Pick<SolveOptions, "searchRadius">): string {
  const radius = decimalFieldState(options.searchRadius, SEARCH_RADIUS_FIELD).value;
  return radius === null ? DEFAULT_SEARCH_RADIUS_TEXT : String(radius);
}

export function sipOrderCap(options: Pick<SolveOptions, "maxSipOrder">): MaxSipOrder | undefined {
  return options.maxSipOrder === "auto" ? undefined : options.maxSipOrder;
}

function usable(report: CatalogDatasetReport): boolean {
  return report.status.state === "installed" || (report.status.state === "external" && report.status.valid);
}

export function offlineCatalogueNames(reports: readonly CatalogDatasetReport[] | null): string[] | null {
  if (reports === null) return null;
  return OFFLINE_CATALOGUES.filter(([id]) => reports.some((report) => report.id === id && usable(report))).map(([, name]) => name);
}

export function twomassMissing(catalogues: readonly string[] | null): boolean {
  return catalogues !== null && !catalogues.includes(TWOMASS_NAME);
}

function offlinePart(catalogues: readonly string[] | null, useTwomass: boolean): string {
  if (catalogues === null) return "Offline catalogues";
  const used = useTwomass ? catalogues : catalogues.filter((name) => name !== TWOMASS_NAME);
  return used.length === 0 ? "No offline catalogue" : `Offline: ${used.join(" + ")}`;
}

export function solveOptionsSummary(options: SolveOptions, catalogues: readonly string[] | null): string {
  const sipCap = sipOrderCap(options);
  return [
    offlinePart(catalogues, options.useTwomass),
    options.useTwomass ? null : `${TWOMASS_NAME} off`,
    options.onlineGaia ? "then online Gaia DR3" : "online off",
    sipCap === undefined ? null : `SIP up to ${sipCap}`,
    options.offerNova ? "astrometry.net offered after a failure" : "no astrometry.net",
  ]
    .filter((part): part is string => part !== null)
    .join(", ");
}
