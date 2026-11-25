import type { SaguiConfig, SaguiInputInfo, SaguiPlaneDqFlags, SaguiRegion, SaguiResult } from "../../shared/types/sagui";
import { targetBand } from "./saguiInputPsfModel";

export const EXACT_WARD_PIXEL_LIMIT = 65536;
export const HST_EXCLUSION_MASK = 0x3fff;
export const DO_NOT_USE_MASK = 1;

export const DQ_PRESETS: readonly { bits: number; label: string }[] = [
  { bits: DO_NOT_USE_MASK, label: "DO_NOT_USE only (1; REED_SOLOMON on HST planes)" },
  { bits: HST_EXCLUSION_MASK, label: "Cube Prep masks for HST and JWST (16383 = 0x3FFF)" },
  { bits: 4294967295, label: "All flags (4294967295)" },
  { bits: 0, label: "Ignore DQ flags (0)" },
];

const JWST_FLAGS = ["DO_NOT_USE", "SATURATED", "JUMP_DET", "DROPOUT", "OUTLIER", "PERSISTENCE", "AD_FLOOR", "CHARGELOSS", "UNRELIABLE_ERROR", "NON_SCIENCE", "DEAD", "HOT", "WARM", "LOW_QE", "RC", "TELEGRAPH", "NONLINEAR", "BAD_REF_PIXEL", "NO_FLAT_FIELD", "NO_GAIN_VALUE", "NO_LIN_CORR", "NO_SAT_CHECK", "UNRELIABLE_BIAS", "UNRELIABLE_DARK", "UNRELIABLE_SLOPE", "UNRELIABLE_FLAT", "OPEN", "ADJ_OPEN", "FLUX_ESTIMATED", "MSA_FAILED_OPEN", "OTHER_BAD_PIXEL", "REFERENCE_PIXEL"];
const ROMAN_FLAGS = ["DO_NOT_USE", "SATURATED", "JUMP_DET", "DROPOUT", "GW_AFFECTED_DATA", "PERSISTENCE", "AD_FLOOR", "OUTLIER", "UNRELIABLE_ERROR", "NON_SCIENCE", "DEAD", "HOT", "WARM", "LOW_QE", "", "TELEGRAPH", "NONLINEAR", "BAD_REF_PIXEL", "NO_FLAT_FIELD", "NO_GAIN_VALUE", "NO_LIN_CORR", "NO_SAT_CHECK", "UNRELIABLE_BIAS", "UNRELIABLE_DARK", "UNRELIABLE_SLOPE", "UNRELIABLE_FLAT", "", "", "UNRELIABLE_RESET", "", "OTHER_BAD_PIXEL", "REFERENCE_PIXEL"];
const HST_FLAGS = ["REED_SOLOMON", "REPLACED_FILL", "BAD_DETECTOR", "MASKED", "HOT", "CTE_TAIL", "WARM", "BAD_BIAS", "SATURATED", "BAD_FLAT", "CHARGE_TRAP", "ATOD_SATURATED", "COSMIC_RAY", "COSMIC_RAY_SINGLE", "MANUAL"];
const FLAG_NAMES: Record<string, readonly string[]> = { jwst: JWST_FLAGS, roman: ROMAN_FLAGS, hst: HST_FLAGS };
const TABLE_NAMES: Record<string, string> = { jwst: "JWST", roman: "Roman", hst: "HST" };

export interface DqFlagRow { name: string; count: number; excluded: boolean }
export interface DqPlaneRow { filter: string; table: string; flags: DqFlagRow[] }
export interface PanelNote { tone: "ok" | "warn"; text: string }

export function psfTargetWarning(input: SaguiInputInfo, psfTarget: string): string | null {
  if (!psfTarget.trim() || targetBand(input, psfTarget) !== null) return null;
  return `${psfTarget.trim()} is not a band of this cube (${input.bands.map((band) => band.filter).join(", ")}). The PSF checks cannot use it; name a PSF outside the cube only when every band was matched to it outside Cube Prep.`;
}

function tableName(table: string): string {
  return TABLE_NAMES[table] ?? "unknown";
}

function flagBit(table: string, name: string): number | null {
  const raw = /^BIT(\d+)$/.exec(name);
  if (raw) return 2 ** Number(raw[1]);
  const index = (FLAG_NAMES[table] ?? []).indexOf(name);
  return index < 0 ? null : 2 ** index;
}

function holdsBit(bits: number, bit: number): boolean {
  return Math.floor(bits / bit) % 2 === 1;
}

export function recommendedDqMask(flags: readonly SaguiPlaneDqFlags[] | undefined): number | null {
  if (!flags || flags.length === 0) return null;
  return flags.reduce((mask, plane) => mask | (plane.table === "hst" ? HST_EXCLUSION_MASK : DO_NOT_USE_MASK), 0);
}

export function dqPlaneRows(flags: readonly SaguiPlaneDqFlags[] | undefined, bits: number): DqPlaneRow[] {
  return (flags ?? []).map((plane) => ({
    filter: plane.filter,
    table: tableName(plane.table),
    flags: Object.entries(plane.flagged_finite)
      .map(([name, count]) => ({ name, count, bit: flagBit(plane.table, name) }))
      .sort((a, b) => (a.bit ?? Infinity) - (b.bit ?? Infinity))
      .map(({ name, count, bit }) => ({ name, count, excluded: bit !== null && holdsBit(bits, bit) })),
  }));
}

export function dqAdvice(flags: readonly SaguiPlaneDqFlags[] | undefined, bits: number): { recommended: number; text: string } | null {
  const recommended = recommendedDqMask(flags);
  if (recommended === null || !flags) return null;
  const groups = new Map<string, string[]>();
  for (const plane of flags) if (TABLE_NAMES[plane.table]) groups.set(tableName(plane.table), [...(groups.get(tableName(plane.table)) ?? []), plane.filter]);
  const mixed = groups.size > 1;
  const hst = flags.some((plane) => plane.table === "hst");
  const others = flags.some((plane) => plane.table !== "hst");
  const missing = Array.from({ length: 14 }, (_, n) => 2 ** n).some((bit) => holdsBit(recommended, bit) && !holdsBit(bits, bit));
  if (!mixed && !missing) return null;
  const parts: string[] = [];
  if (mixed) parts.push(`The planes use different DQ tables (${[...groups].map(([table, filters]) => `${table}: ${filters.join(", ")}`).join("; ")}) and one mask applies to all of them.`);
  if (missing) {
    const usage = [hst ? "0x3FFF (REED_SOLOMON to COSMIC_RAY_SINGLE) on HST planes" : null, others ? "DO_NOT_USE (bit 0) on the other planes" : null].filter(Boolean).join(" and ");
    parts.push(`Cube Prep excludes ${usage}; ${hst && others ? "their union is" : "the same flags here need"} ${recommended}.`);
  } else if (bits === recommended && hst && others) {
    parts.push(`${recommended} is the union of the masks Cube Prep uses, so on the JWST and Roman planes it also excludes SATURATED to LOW_QE (bits 2 to 8192).`);
  }
  return { recommended, text: parts.join(" ") };
}

export function runtimeAdvice(issue: string): string | null {
  const version = /^Expected R (\d+\.\d+\.\d+)/.exec(issue);
  if (version) return `install R ${version[1]} alongside your current R (the Windows installer is in the CRAN archive) and choose its Rscript.`;
  if (issue === "Expected the pinned SAGUI commit" || issue === "SAGUI does not match the pinned upstream commit") return "use Install / repair SAGUI library to build the pinned commit into a new private library.";
  if (issue === "SAGUI is missing or a dependency could not be loaded") return "use Install / repair SAGUI library to install SAGUI and its dependencies into a new private library.";
  if (issue === "SAGUI must be in the private library") return "choose the private library that holds SAGUI, or use Install / repair SAGUI library.";
  const excluded = /^CPU profile excludes (\S+)/.exec(issue);
  if (excluded) return `the library holds ${excluded[1]}, which the CPU profile leaves out; use Install / repair SAGUI library to make a clean private library.`;
  return null;
}

export function runtimeIssueLine(issue: string): string {
  const advice = runtimeAdvice(issue);
  return advice ? `${issue}: ${advice}` : issue;
}

export function runtimeIssuesText(issues: readonly string[]): string {
  return issues.map(runtimeIssueLine).join(" ");
}

function count(value: number): string {
  return value.toLocaleString("en-US");
}

export function wardVerdict(input: Pick<SaguiInputInfo, "width" | "height">, mode: SaguiConfig["mode"]): PanelNote | null {
  if (mode !== "ward") return null;
  const pixels = input.width * input.height;
  return pixels <= EXACT_WARD_PIXEL_LIMIT
    ? { tone: "ok", text: `Exact Ward: the full footprint of ${count(pixels)} pixels is within its ${count(EXACT_WARD_PIXEL_LIMIT)} pixel limit.` }
    : { tone: "warn", text: `Exact Ward refuses more than ${count(EXACT_WARD_PIXEL_LIMIT)} support pixels and the full footprint has ${count(pixels)}. The starlet and DQ masks may bring the support under the limit; otherwise crop the field in Cube Prep or choose Sparse Ward.` };
}

export function resultSummary(result: Pick<SaguiResult, "regions" | "warnings">): string {
  const support = result.regions.reduce((total, region) => total + region.n_pixels, 0);
  const warnings = result.warnings.length === 0 ? "no warnings" : result.warnings.length === 1 ? "1 warning" : `${result.warnings.length} warnings`;
  return `${result.regions.length} ${result.regions.length === 1 ? "region" : "regions"}, ${count(support)} support pixels, ${warnings}`;
}

function csvNumber(value: number | null | undefined): string {
  return typeof value === "number" && Number.isFinite(value) ? String(value) : "";
}

export function regionSedCsv(region: SaguiRegion, unit: string): string {
  const rows = region.bands.map((band) => [band.filter, csvNumber(band.pivot_um), csvNumber(band.flux), csvNumber(band.error), csvNumber(band.error_correlated)].join(","));
  return `# SAGUI region ${region.region}, ${region.n_pixels} pixels, flux unit ${unit || "input flux units"}\nfilter,pivot_um,flux,error,error_correlated\n${rows.join("\n")}\n`;
}
