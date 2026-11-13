import type { FitsExtension } from "../services/header";
import type { PlaneInfo } from "../shared/types/fits.types";
import type { BandPlaneEntry, CompanionRole, ErrorLayerRole, LayerRole } from "../shared/types/layers";
import { parseBandPlaneRef, parseImageRef, planeLabel, sourceFile } from "./imageRef";

export type ChipTone = "dq" | "err";

export interface RoleChip {
  label: string;
  title: string;
  tone: ChipTone;
}

export interface RowActions {
  asErr: boolean;
  asDq: boolean;
  inUse: "err" | "dq" | null;
}

export interface PendingLayer {
  role: CompanionRole;
  source: string;
  name: string;
}

export interface LayerLine {
  role: CompanionRole;
  title: "Error" | "Mask";
  state: "in_use" | "not_used" | "none";
  text: string;
  chip: string | null;
  detail: string | null;
  canSwitchOff: boolean;
  canAutomatic: boolean;
}

export const ERROR_KIND_OPTIONS: readonly { value: ErrorLayerRole | "auto"; label: string }[] = [
  { value: "auto", label: "Automatic (from its name or cards)" },
  { value: "sigma", label: "Sigma (as is)" },
  { value: "variance", label: "Variance (square root)" },
  { value: "inverse_variance", label: "Inverse variance (1 / square root)" },
];

const NO_LAYER_BASIS = "no layer name or card";
const SWITCHED_OFF = "switched off by the user";
const NONE_FOUND = "none found in this file or beside it";
const ERROR_ROLES: readonly LayerRole[] = ["sigma", "variance", "inverse_variance"];
const NO_ACTIONS: RowActions = { asErr: false, asDq: false, inUse: null };

const ORIGIN_LABELS = new Map<string, string>([
  ["same_file", "same file"],
  ["sibling", "sibling file"],
  ["user", "set by hand"],
  ["recipe", "rebuilt"],
]);

type ChipSource = Pick<FitsExtension, "extname" | "is_dq" | "is_err" | "role" | "role_basis" | "role_assumed">;

function roleWords(role: LayerRole, assumed: boolean): string | null {
  switch (role) {
    case "mask":
      return "mask";
    case "sigma":
      return "sigma (error)";
    case "variance":
      return "variance";
    case "inverse_variance":
      return assumed ? "inverse variance (assumed)" : "inverse variance";
    default:
      return null;
  }
}

function isDqName(extname: string | null): boolean {
  const name = (extname ?? "").trim().toUpperCase();
  return name === "DQ" || name.endsWith(".DQ");
}

function chipLabel(ext: ChipSource, role: LayerRole): string | null {
  switch (role) {
    case "mask":
      return isDqName(ext.extname) ? "DQ" : "MASK";
    case "sigma":
      return ext.is_err ? "ERR" : "SIGMA";
    case "variance":
      return "VAR";
    case "inverse_variance":
      return ext.role_assumed ? "IVAR?" : "IVAR";
    default:
      return null;
  }
}

export function roleChip(ext: ChipSource): RoleChip | null {
  if (ext.role === undefined) {
    if (ext.is_dq) return { label: "DQ", title: "DQ layer", tone: "dq" };
    if (ext.is_err) return { label: "ERR", title: "ERR layer", tone: "err" };
    return null;
  }
  const label = chipLabel(ext, ext.role);
  const words = roleWords(ext.role, ext.role_assumed === true);
  if (label === null || words === null) return null;
  return { label, title: `${words} layer, from ${ext.role_basis}`, tone: ext.role === "mask" ? "dq" : "err" };
}

export function bandButtonLabel(band: Pick<BandPlaneEntry, "plane" | "label">): string {
  return band.label ? band.label : `plane ${band.plane}`;
}

function holdsLayer(layerRef: string | null, rowRef: string): boolean {
  return layerRef !== null && (layerRef === rowRef || layerRef.startsWith(`${rowRef}&plane=`));
}

export function rowActions(
  ext: Pick<FitsExtension, "kind" | "has_data" | "ref" | "role" | "role_basis"> & Partial<Pick<FitsExtension, "naxis" | "naxis3">>,
  plane: Pick<PlaneInfo, "err_ref" | "dq_ref"> | null,
  shown: boolean,
): RowActions {
  if (plane === null || shown || ext.kind !== "hdu" || !ext.has_data) return NO_ACTIONS;
  const inUse = holdsLayer(plane.err_ref, ext.ref) ? "err" : holdsLayer(plane.dq_ref, ext.ref) ? "dq" : null;
  const cube = (ext.naxis ?? 0) >= 3 && (ext.naxis3 ?? 0) > 1;
  const role = ext.role ?? "other";
  const unnamed = role === "other" || (role === "science" && ext.role_basis === NO_LAYER_BASIS);
  return {
    asErr: !cube && inUse !== "err" && (unnamed || ERROR_ROLES.includes(role)),
    asDq: !cube && inUse !== "dq" && (unnamed || role === "mask"),
    inUse,
  };
}

export function hduLayerName(ext: Pick<FitsExtension, "index" | "extname">): string {
  return ext.extname ? `HDU ${ext.index} (${ext.extname})` : `HDU ${ext.index}`;
}

export function fileLayerName(path: string): string {
  const file = sourceFile(path);
  const name = file.split(/[\\/]/).at(-1) ?? file;
  const band = parseBandPlaneRef(path);
  if (band) return `${name} HDU ${band.hdu} plane ${band.plane}`;
  const plane = planeLabel(parseImageRef(path));
  return plane === null ? name : `${name} ${plane}`;
}

export function originLabel(origin: string | null | undefined): string | null {
  return origin ? ORIGIN_LABELS.get(origin) ?? null : null;
}

export function noDataText(n: number | null | undefined): string | null {
  if (!n) return null;
  return n === 1 ? "1 pixel without data" : `${n} pixels without data`;
}

export function shownLayerText(plane: Pick<PlaneInfo, "layer">): string | null {
  const layer = plane.layer;
  const words = layer ? roleWords(layer.role, layer.assumed) : null;
  if (!layer || words === null) return null;
  return `The shown plane is a layer itself: ${words} (${layer.basis}). Layers are assigned to science images.`;
}

const handOrigin = (origin: string | null | undefined) => origin === "user" || origin === "recipe";
const switchedOff = (reason: string | null | undefined) => typeof reason === "string" && reason.endsWith(SWITCHED_OFF);

function unusedLine(role: CompanionRole, title: LayerLine["title"], reason: string | null | undefined): LayerLine {
  const base = { role, title, chip: null, detail: null, canSwitchOff: false };
  if (typeof reason !== "string") return { ...base, state: "none", text: NONE_FOUND, canAutomatic: false };
  return { ...base, state: "not_used", text: reason, canAutomatic: switchedOff(reason) };
}

function errorLine(plane: PlaneInfo): LayerLine {
  if (plane.err_ref === null) return unusedLine("err", "Error", plane.err_unavailable);
  const origin = plane.err_origin;
  return {
    role: "err",
    title: "Error",
    state: "in_use",
    text: origin?.source ?? plane.err_ref,
    chip: originLabel(origin?.origin),
    detail: noDataText(origin?.n_no_data),
    canSwitchOff: true,
    canAutomatic: handOrigin(origin?.origin),
  };
}

function maskLine(plane: PlaneInfo): LayerLine {
  if (plane.dq_ref === null) return unusedLine("dq", "Mask", plane.dq_unavailable);
  return {
    role: "dq",
    title: "Mask",
    state: "in_use",
    text: plane.dq_origin ?? plane.dq_ref,
    chip: originLabel(plane.dq_origin_kind),
    detail: null,
    canSwitchOff: true,
    canAutomatic: handOrigin(plane.dq_origin_kind),
  };
}

export function layerLines(plane: PlaneInfo): [LayerLine, LayerLine] {
  return [errorLine(plane), maskLine(plane)];
}

export function handSet(plane: PlaneInfo): boolean {
  return (
    handOrigin(plane.err_origin?.origin) ||
    plane.dq_origin_kind === "user" ||
    switchedOff(plane.err_unavailable) ||
    switchedOff(plane.dq_unavailable)
  );
}
