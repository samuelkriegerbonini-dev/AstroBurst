import type { StfParams } from "../shared/types";
import { exportStem, parseImageRef } from "./imageRef";

export function exportTimestamp(date: Date): string {
  return date.toISOString().replace(/[-:]/g, "").replace("T", "-").replace(".", "-").slice(0, 19);
}

export function planeTag(pathOrRef: string): string {
  const { plane } = parseImageRef(pathOrRef);
  switch (plane.kind) {
    case "auto":
      return "";
    case "hdu":
      return `_hdu${plane.index}`;
    case "array":
      return `_${plane.key.replace(/[^A-Za-z0-9._-]/g, "_")}`;
  }
}

export function exportFileName(pathOrRef: string, suffix: string, ext: string, stamp: string): string {
  return `${exportStem(pathOrRef)}${planeTag(pathOrRef)}${suffix}_${stamp}.${ext}`;
}

export const COMPOSITE_EXPORT_STEM = "astroburst_composite";

export function compositeExportName(suffix: string, ext: string, stamp: string): string {
  return `${COMPOSITE_EXPORT_STEM}${suffix}_${stamp}.${ext}`;
}

export interface RgbChannelPaths {
  r: string | null;
  g: string | null;
  b: string | null;
}

export type RgbExportSource =
  | { kind: "channels"; channels: RgbChannelPaths }
  | { kind: "composite" }
  | { kind: "file"; path: string };

export interface RgbExportInput {
  rgbChannels: RgbChannelPaths | null;
  compositePreviewUrl: string | null;
  filePath: string | null;
  fileIsRgb: boolean;
  filePreviewUrl: string | null;
}

export function resolveRgbExportSource(input: RgbExportInput): RgbExportSource | null {
  const showsFileRgb = input.compositePreviewUrl === null || input.compositePreviewUrl === input.filePreviewUrl;
  if (input.fileIsRgb && input.filePath && showsFileRgb) return { kind: "file", path: input.filePath };
  if (input.compositePreviewUrl !== null) return { kind: "composite" };
  const ch = input.rgbChannels;
  if (ch && (ch.r || ch.g || ch.b)) return { kind: "channels", channels: ch };
  return null;
}

function baseName(path: string): string {
  return path.split(/[/\\]/).pop() || path;
}

export function channelSourceLabel(ch: RgbChannelPaths): string {
  const part = (name: string, p: string | null) => `${name}=${p ? baseName(p) : "none"}`;
  return `Source: Headers-tab channels ${part("R", ch.r)}, ${part("G", ch.g)}, ${part("B", ch.b)}`;
}

export function rgbCubeUnavailableReason(filePath: string | null, channels: RgbChannelPaths | null): string | null {
  if (filePath) return null;
  if (channels && (channels.r || channels.g || channels.b) && !(channels.r && channels.g && channels.b)) {
    return "Assign R, G and B in the Headers tab to export an RGB FITS cube";
  }
  return null;
}

export function rgbExportPaths(filePath: string | null, channels: RgbChannelPaths | null): RgbChannelPaths {
  if (filePath) return { r: filePath, g: filePath, b: filePath };
  return { r: channels?.r ?? null, g: channels?.g ?? null, b: channels?.b ?? null };
}

export interface RgbPngStfArgs {
  applyStfStretch: boolean;
  linked: boolean;
  shadowR?: number;
  midtoneR?: number;
  highlightR?: number;
  shadowG?: number;
  midtoneG?: number;
  highlightG?: number;
  shadowB?: number;
  midtoneB?: number;
  highlightB?: number;
}

export function fileRgbPngStf(stf: { r: StfParams; g: StfParams; b: StfParams; linked: boolean } | null): RgbPngStfArgs {
  if (!stf) return { applyStfStretch: false, linked: false };
  return compositeRgbPngStf(stf, true, stf.linked);
}

export function compositeRgbPngStf(
  stf: { r: StfParams; g: StfParams; b: StfParams } | null,
  applyStf: boolean,
  linked: boolean,
): RgbPngStfArgs {
  if (!stf) return { applyStfStretch: applyStf, linked };
  return {
    applyStfStretch: true,
    linked,
    shadowR: stf.r.shadow,
    midtoneR: stf.r.midtone,
    highlightR: stf.r.highlight,
    shadowG: stf.g.shadow,
    midtoneG: stf.g.midtone,
    highlightG: stf.g.highlight,
    shadowB: stf.b.shadow,
    midtoneB: stf.b.midtone,
    highlightB: stf.b.highlight,
  };
}

export interface MefHduReport {
  dropped: readonly string[];
  kept_raw: readonly string[];
  uncompressed: readonly string[];
}

export function mefHduSummary(report: MefHduReport): string[] {
  const list = (names: readonly string[]) => (names.length > 0 ? names.join(", ") : "none");
  const lines = [`dropped: ${list(report.dropped)}`, `kept raw: ${list(report.kept_raw)}`];
  if (report.uncompressed.length > 0) {
    lines.push(`copied uncompressed (codec cannot take them): ${report.uncompressed.join(", ")}`);
  }
  return lines;
}

export interface DisplayedSource {
  isProcessed: boolean;
  label: string | null;
  previewOnly: boolean;
}

export function exportSourceLabel(d: DisplayedSource): string {
  if (!d.isProcessed) return "original file";
  const label = d.label ?? "result";
  if (d.previewOnly) return `original file (${label} is PNG-only)`;
  return `processed: ${label}`;
}

const ASSET_PREFIXES = ["asset://localhost/", "http://asset.localhost/", "https://asset.localhost/"];

export function assetUrlToPath(url: string): string | null {
  const prefix = ASSET_PREFIXES.find((p) => url.startsWith(p));
  if (!prefix) return null;
  const rest = url.slice(prefix.length);
  const end = rest.search(/[?#]/);
  const encoded = end >= 0 ? rest.slice(0, end) : rest;
  if (encoded.length === 0) return null;
  try {
    return decodeURIComponent(encoded);
  } catch {
    return null;
  }
}

export function zipEntryName(fileName: string, taken: Set<string>): string {
  const stem = exportStem(fileName, "image");
  let name = `${stem}.png`;
  for (let n = 2; taken.has(name.toLowerCase()); n++) name = `${stem}_${n}.png`;
  taken.add(name.toLowerCase());
  return name;
}

export interface ZipCandidate {
  path: string;
  source: string;
}

export interface ZipFileInput {
  pngPath: string | null;
  processed: { previewUrl: string | null; label: string } | null;
}

export function zipCandidates(input: ZipFileInput): ZipCandidate[] {
  const out: ZipCandidate[] = [];
  const processed = input.processed;
  const processedPath = processed?.previewUrl ? assetUrlToPath(processed.previewUrl) : null;
  if (processed && processedPath) out.push({ path: processedPath, source: `processed (${processed.label})` });
  if (input.pngPath) {
    const source = processed
      ? `original preview (processed result ${processed.label} not included)`
      : "original preview";
    out.push({ path: input.pngPath, source });
  }
  return out;
}

export function manifestLine(entry: string | null, source: string, filePath: string): string {
  return `${entry ?? "-"}\t${source}\t${filePath}`;
}
