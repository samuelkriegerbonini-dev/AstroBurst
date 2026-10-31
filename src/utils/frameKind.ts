export type FrameKind = "bias" | "dark" | "flat" | "flatdark" | "light" | "unknown";

export type CalibrationFrameKind = Exclude<FrameKind, "light" | "unknown">;

export type CalibrationFrames<T> = Record<CalibrationFrameKind, T[]>;

export interface FrameKindSource {
  result?: { header?: Record<string, string> | null } | null;
}

export interface FramesByKind<T> {
  lights: T[];
  calibration: CalibrationFrames<T>;
}

const CALIBRATION_FRAME_KINDS: readonly CalibrationFrameKind[] = ["dark", "flat", "flatdark", "bias"];

export const FRAME_KIND_CHIPS: Record<FrameKind, string | null> = {
  light: "L",
  dark: "D",
  flat: "F",
  bias: "B",
  flatdark: "FD",
  unknown: null,
};

export const FRAME_KIND_NAMES: Record<FrameKind, string> = {
  light: "light",
  dark: "dark",
  flat: "flat",
  bias: "bias",
  flatdark: "flat dark",
  unknown: "unknown",
};

const IMAGETYP_KEYS = ["IMAGETYP", "FRAMETYP"] as const;

const LEFT_OUT_NOUNS: Record<CalibrationFrameKind, readonly [string, string]> = {
  dark: ["dark", "darks"],
  flat: ["flat", "flats"],
  flatdark: ["flat dark", "flat darks"],
  bias: ["bias", "bias"],
};

function cardText(raw: string | undefined): string | null {
  if (raw == null) return null;
  const text = String(raw).trim().replace(/^'+|'+$/g, "").trim();
  return text === "" ? null : text;
}

function imagetypOf(header: Record<string, string> | null | undefined): string | null {
  if (!header) return null;
  for (const key of IMAGETYP_KEYS) {
    const text = cardText(header[key]);
    if (text !== null) return text;
  }
  return null;
}

function asciiLowercase(text: string): string {
  return text.replace(/[A-Z]+/g, (run) => run.toLowerCase());
}

export function classifyImagetyp(imagetyp: string | null | undefined): FrameKind {
  if (imagetyp == null) return "unknown";
  const text = asciiLowercase(imagetyp);
  if (text.includes("bias") || text.includes("zero")) return "bias";
  if (text.includes("flat") && text.includes("dark")) return "flatdark";
  if (text.includes("flat")) return "flat";
  if (text.includes("dark")) return "dark";
  if (text.includes("light") || text.includes("object")) return "light";
  return "unknown";
}

export function frameKindOf(header: Record<string, string> | null | undefined): FrameKind {
  return classifyImagetyp(imagetypOf(header));
}

export function isCalibrationKind(kind: FrameKind): kind is CalibrationFrameKind {
  return kind !== "light" && kind !== "unknown";
}

export function sortByFrameKind<T extends FrameKindSource>(files: readonly T[]): FramesByKind<T> {
  const sorted: FramesByKind<T> = { lights: [], calibration: { dark: [], flat: [], flatdark: [], bias: [] } };
  for (const file of files) {
    const kind = frameKindOf(file.result?.header);
    if (isCalibrationKind(kind)) sorted.calibration[kind].push(file);
    else sorted.lights.push(file);
  }
  return sorted;
}

export function calibrationPaths<T extends { path: string }>(calibration: CalibrationFrames<T>): CalibrationFrames<string> {
  return {
    dark: calibration.dark.map((f) => f.path),
    flat: calibration.flat.map((f) => f.path),
    flatdark: calibration.flatdark.map((f) => f.path),
    bias: calibration.bias.map((f) => f.path),
  };
}

export function imagetypLeftOutLine(calibration: Readonly<Record<CalibrationFrameKind, readonly unknown[]>>): string | null {
  const counted = CALIBRATION_FRAME_KINDS.map((kind) => [kind, calibration[kind].length] as const).filter(([, n]) => n > 0);
  if (counted.length === 0) return null;
  const total = counted.reduce((sum, [, n]) => sum + n, 0);
  const parts = counted.map(([kind, n]) => `${n} ${LEFT_OUT_NOUNS[kind][n === 1 ? 0 : 1]}`);
  return `${total} frame${total === 1 ? "" : "s"} left out by IMAGETYP: ${parts.join(", ")}`;
}

const STACKED_PROVENANCE: ReadonlySet<string> = new Set(["stacked", "drizzled", "rejection"]);

export function isStackedProduct(header: Record<string, string> | null | undefined): boolean {
  if (!header) return false;
  if (cardText(header.ABCOMB) !== null) return true;
  const provenance = cardText(header.ABPROC);
  return provenance !== null && STACKED_PROVENANCE.has(asciiLowercase(provenance));
}

export function stackedLeftOutLine(count: number, leftOutOf: string): string | null {
  if (count === 0) return null;
  return `${count} stacked result${count === 1 ? "" : "s"} left out of ${leftOutOf}`;
}

export function subframeLights<T extends FrameKindSource & { path: string }>(files: readonly T[]): { paths: string[]; leftOutLine: string | null } {
  const sorted = sortByFrameKind(files);
  const lights = sorted.lights.filter((file) => !isStackedProduct(file.result?.header));
  const lines = [imagetypLeftOutLine(sorted.calibration), stackedLeftOutLine(sorted.lights.length - lights.length, "the analysis")];
  const said = lines.filter((line): line is string => line !== null);
  return { paths: lights.map((file) => file.path), leftOutLine: said.length > 0 ? said.join("; ") : null };
}
