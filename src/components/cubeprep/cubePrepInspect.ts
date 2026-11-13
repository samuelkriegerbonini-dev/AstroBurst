import type { FitsExtension } from "../../services/header";
import type { CompanionSuggestions, LayerRole } from "../../shared/types/layers";
import type { ImagePsfFwhm, ResolutionCheck, ResolutionRequest, ResolutionSettings } from "../../shared/types/resolution";
import { pixelScaleText } from "../../utils/coordFormat";
import { imagePsfText, RESOLUTION_LIMITS, resolutionRequest, scaleText, significant } from "../../utils/resolutionText";
import { fileBaseName } from "../../utils/values";

export const FITS_INPUT = /\.(fits|fit|fts)(\.(gz|bz2|fz))?$/i;
export const FITS_INPUT_EXTENSIONS = ["fits", "fit", "fts", "gz", "bz2", "fz"];
export const REDSHIFT_HELP = "Written to the cube header as REDSHIFT. The SAGUI check reads it back and gives the PSF in kpc; Cube Prep itself does not use it.";
export const COMMON_PSF_REFERENCE = "target PSF of the last Cube Prep run";

const PART_SEPARATOR = " · ";
const NOT_EVERY_LAYER_IN_USE = "not every layer listed is in use for this file";
const ERROR_KIND: Partial<Record<LayerRole, string>> = { sigma: "sigma", variance: "variance", inverse_variance: "inverse variance" };

export function inspectionKeyOf(draft: { baseDir: string; recipe: { inputs: unknown } }): string {
  return JSON.stringify([draft.baseDir, draft.recipe.inputs]);
}

function extensionName(extension: FitsExtension): string {
  return extension.extname ? `${extension.extname}, HDU ${extension.index}` : `HDU ${extension.index}`;
}

function splitReference(reference: string): [string, string] {
  const hash = reference.indexOf("#");
  return hash < 0 ? [reference, ""] : [reference.slice(0, hash), reference.slice(hash + 1)];
}

function comparableFile(file: string): string {
  return file.replace(/\\/g, "/").toLowerCase();
}

export function layerName(source: string, inputPath: string, extensions: readonly FitsExtension[]): string {
  const own = extensions.find((extension) => extension.ref === source);
  if (own) return extensionName(own);
  const [file, fragment] = splitReference(source);
  const [inputFile] = splitReference(inputPath);
  const hduText = /(?:^|&)hdu=(\d+)/.exec(fragment)?.[1];
  const hdu = hduText === undefined ? null : Number(hduText);
  if (comparableFile(file) === comparableFile(inputFile)) return hdu === null ? fileBaseName(file) : `HDU ${hdu}`;
  return hdu === null ? fileBaseName(file) : `${fileBaseName(file)}, HDU ${hdu}`;
}

function errorKind(role: LayerRole): string {
  return ERROR_KIND[role] ?? role;
}

function declaredErrorPart(extensions: readonly FitsExtension[]): string {
  if (extensions.some((extension) => extension.is_err)) return "ERR present";
  const layer = extensions.find((extension) => extension.role !== undefined && ERROR_KIND[extension.role] !== undefined);
  return layer?.role === undefined ? "ERR absent" : `error: ${extensionName(layer)} (${errorKind(layer.role)})`;
}

function declaredMaskPart(extensions: readonly FitsExtension[]): string {
  if (extensions.some((extension) => extension.is_dq)) return "DQ present";
  const layer = extensions.find((extension) => extension.role === "mask");
  return layer ? `mask: ${extensionName(layer)}` : "DQ absent";
}

function refusal<T extends { source: string; usable: boolean }>(layer: { source: string } | null, candidates: readonly T[]): T | undefined {
  const candidate = layer === null ? undefined : candidates.find((entry) => entry.source === layer.source);
  return candidate !== undefined && !candidate.usable ? candidate : undefined;
}

function unusedText(kind: "error" | "mask", name: string, reason: string | null): string {
  return `${kind} layer ${name} not used${reason ? `: ${reason}` : ""}`;
}

export function layersText(inputPath: string, unit: string | null | undefined, extensions: readonly FitsExtension[], layers: CompanionSuggestions | null): string {
  const unitPart = unit || "unit missing";
  if (layers === null) return [unitPart, declaredErrorPart(extensions), declaredMaskPart(extensions)].join(PART_SEPARATOR);
  const { err, dq, candidates } = layers;
  const errRefusal = refusal(err, candidates.err);
  const dqRefusal = refusal(dq, candidates.dq);
  const parts = [unitPart];
  if (err === null) parts.push("ERR absent");
  else {
    const name = layerName(err.source, inputPath, extensions);
    parts.push(errRefusal ? unusedText("error", name, errRefusal.reason) : `error: ${name} (${errorKind(err.role)})`);
  }
  if (dq === null) parts.push("DQ absent");
  else {
    const name = layerName(dq.source, inputPath, extensions);
    parts.push(dqRefusal ? unusedText("mask", name, dqRefusal.reason) : `mask: ${name}`);
  }
  if (!layers.applied && (err !== null || dq !== null) && !errRefusal && !dqRefusal) parts.push(NOT_EVERY_LAYER_IN_USE);
  return parts.join(PART_SEPARATOR);
}

export function scaleLine(pixelScaleArcsec: number | null | undefined): string {
  const scale = pixelScaleText(pixelScaleArcsec);
  return scale === null ? "No celestial WCS: pixel scale unknown" : `Pixel scale ${scale}`;
}

export function psfLine(psf: ImagePsfFwhm): string {
  return `PSF FWHM ${imagePsfText(psf)}`;
}

export interface PsfFwhmRow {
  filter: string;
  sourcePx: number | null;
  targetPx: number | null;
  matchedPx: number | null;
  scaleArcsec: number | null;
  reference: boolean;
}

type StepReport = { step: string; report: unknown };
type Fields = Record<string, unknown>;
type NamedEntry = Fields & { filter: string };

function isFields(value: unknown): value is Fields {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function num(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) && value > 0 ? value : null;
}

function sameFilter(a: string, b: string): boolean {
  return a.trim().toLowerCase() === b.trim().toLowerCase();
}

function lastStepList(steps: readonly StepReport[], name: string, listOf: (report: Fields) => unknown): { report: Fields; entries: NamedEntry[] } | null {
  for (let index = steps.length - 1; index >= 0; index--) {
    const { step, report } = steps[index];
    if (step !== name || !isFields(report)) continue;
    const list = listOf(report);
    if (!Array.isArray(list)) continue;
    const entries = list.filter((entry): entry is NamedEntry => isFields(entry) && typeof entry.filter === "string" && entry.filter.trim() !== "");
    return { report, entries };
  }
  return null;
}

export function psfFwhmRows(steps: readonly StepReport[]): PsfFwhmRow[] {
  const kernels = lastStepList(steps, "kernels", (report) => report.kernels);
  const psfs = lastStepList(steps, "psfs", (report) => (isFields(report.prepared) ? report.prepared.psfs : null));
  const rows: PsfFwhmRow[] = [];
  const kernelScale = num(kernels?.report.pixel_scale_arcsec);
  for (const entry of kernels?.entries ?? []) {
    const diagnostics = isFields(entry.diagnostics) ? entry.diagnostics : {};
    rows.push({ filter: entry.filter, sourcePx: num(diagnostics.source_fwhm_px), targetPx: num(diagnostics.target_fwhm_px), matchedPx: num(diagnostics.matched_fwhm_px), scaleArcsec: kernelScale, reference: false });
  }
  const reference = kernels?.report.reference;
  for (const entry of psfs?.entries ?? []) {
    if (rows.some((row) => sameFilter(row.filter, entry.filter))) continue;
    const after = isFields(entry.after) ? entry.after : {};
    rows.push({ filter: entry.filter, sourcePx: num(after.fwhm_px), targetPx: null, matchedPx: null, scaleArcsec: num(entry.pixel_scale_arcsec), reference: typeof reference === "string" && sameFilter(entry.filter, reference) });
  }
  return rows;
}

export function fwhmCell(px: number | null, scaleArcsec: number | null): string {
  if (px === null) return "--";
  return scaleArcsec === null ? `${px.toFixed(2)} px` : `${px.toFixed(2)} px (${(px * scaleArcsec).toFixed(3)}")`;
}

export function commonPsfArcsec(steps: readonly StepReport[]): number | null {
  for (const row of psfFwhmRows(steps)) if (row.targetPx !== null && row.scaleArcsec !== null) return row.targetPx * row.scaleArcsec;
  return null;
}

export interface RedshiftNote {
  scale: string | null;
  psf: string | null;
  cosmology: string;
}

export function redshiftRequest(redshift: unknown, settings: ResolutionSettings, commonPsf: number | null): ResolutionRequest | null {
  if (typeof redshift !== "number" || !Number.isFinite(redshift) || redshift <= 0 || redshift > RESOLUTION_LIMITS.redshiftMax) return null;
  const psf = { fwhmArcsec: commonPsf, halfLightRadiusArcsec: null, reference: commonPsf === null ? null : COMMON_PSF_REFERENCE, missing: null };
  return resolutionRequest({ ...settings, redshift }, psf, null, {});
}

export function redshiftNote(check: ResolutionCheck): RedshiftNote {
  const psf = check.psf_fwhm_arcsec !== null && check.psf_fwhm_kpc !== null ? `${significant(check.psf_fwhm_arcsec)} arcsec = ${significant(check.psf_fwhm_kpc)} kpc` : null;
  return { scale: scaleText(check), psf, cosmology: check.cosmology };
}
