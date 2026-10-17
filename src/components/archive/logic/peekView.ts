import type { MastError, MastMissionId, MastPeek, MastProduct } from "../../../shared/types/mast";
import { NO_FOOTPRINT_TEXT } from "../../../utils/mastFootprints";
import { errorView } from "./errorCopy";
import { formatBytes } from "./productView";

export const PEEK_BUTTON_TEXT = "Fetch footprint";
export const PEEK_BUSY_TEXT = "Reading…";
export const PEEK_LOADING_TEXT = "Reading the FITS header from MAST…";
export const PEEK_TITLE = "Read the image size and footprint from this file's FITS header at MAST (at most 524 KB, no download)";
export const PEEK_HINT = "Fetch footprint in Products reads it from a FITS header";
export const PEEK_TIMEOUT_SEC = 60;

const PEEK_MISSIONS: readonly MastMissionId[] = ["jwst", "hst", "tess"];
const HLSP_PREFIX = "mast:HLSP/";
const FITS_NAME = /\.(fits|fit|fts|fits\.fz)$/i;
const DRIZZLED_NAME = /_(drc|drz)\.(fits|fits\.fz)$/i;
const SEPARATOR = " · ";
const PIXEL_TYPES: Readonly<Record<number, string>> = { [-32]: "float32", [-64]: "float64", 8: "uint8", 16: "int16", 32: "int32", 64: "int64" };

function hlspUri(product: Pick<MastProduct, "mastUri">): string | null {
  return product.mastUri.startsWith(HLSP_PREFIX) ? product.mastUri : null;
}

export function peekableMission(mission: MastMissionId): boolean {
  return PEEK_MISSIONS.includes(mission);
}

export function peekable(product: MastProduct, mission: MastMissionId): boolean {
  if (product.opensAs !== "image" || !FITS_NAME.test(product.filename)) return false;
  return peekableMission(mission) || hlspUri(product) !== null;
}

export function peekUri(product: Pick<MastProduct, "mastUri">): string | null {
  return hlspUri(product);
}

export function peekKey(mission: MastMissionId, datasetId: string, filename: string): string {
  return `${mission}:${datasetId}:${filename}`;
}

export function drizzled(filename: string): boolean {
  return DRIZZLED_NAME.test(filename);
}

export function peekRequestId(datasetId: string, filename: string): string {
  return `peek:${datasetId}:${filename}`;
}

export function peekPolygons(peek: MastPeek): [number, number][][] {
  return peek.footprints.map((footprint) => footprint.corners.map(([ra, dec]): [number, number] => [ra, dec]));
}

export function pixelType(bitpix: number | null): string | null {
  return bitpix === null ? null : (PIXEL_TYPES[bitpix] ?? null);
}

function plural(n: number, one: string, many: string): string {
  return `${n} ${n === 1 ? one : many}`;
}

function sizeText(peek: MastPeek): string | null {
  if (!peek.dims.length) return null;
  const type = pixelType(peek.bitpix);
  const dims = peek.dims.join(" x ");
  return type ? `${dims} ${type}` : dims;
}

function readText(peek: MastPeek): string {
  if (peek.fromCache) return "from the cache";
  return `${formatBytes(peek.bytesFetched)} in ${plural(peek.ranges, "range", "ranges")}`;
}

export function peekSummary(peek: MastPeek): string {
  const parts: (string | null)[] = [
    sizeText(peek),
    peek.hdus.length > 1 ? `${peek.hdus.length} science extensions` : null,
    [peek.instrume, peek.filter].filter((part): part is string => !!part).join(" ") || null,
    peek.exptime !== null && Number.isFinite(peek.exptime) ? `${Number(peek.exptime.toFixed(1))} s` : null,
    peek.footprints.length ? plural(peek.footprints.length, "footprint", "footprints") : "no footprint",
    peek.complete ? null : "partial",
    readText(peek),
  ];
  return parts.filter((part): part is string => !!part).join(SEPARATOR);
}

function sentence(text: string): string {
  const trimmed = text.trim().replace(/[.\s]+$/, "");
  return trimmed ? `${trimmed}.` : "";
}

function capitalized(text: string): string {
  return text ? `${text[0].toUpperCase()}${text.slice(1)}` : text;
}

function statusText(err: MastError): string {
  return err.context.status !== undefined ? `HTTP ${err.context.status}` : "not found";
}

export function peekErrorText(err: MastError): string {
  switch (err.code) {
    case "mast_peek_incomplete":
      return `${capitalized(sentence(err.message))} Download the file to read its headers.`;
    case "mast_timeout":
      return `MAST did not send the header within ${err.context.timeoutSec ?? PEEK_TIMEOUT_SEC} s. Try again in a moment.`;
    case "mast_decode":
      return capitalized(sentence(err.message));
    case "mast_unauthorized":
      return "This file is in exclusive access at MAST.";
    case "mast_not_deployed":
      return `MAST has no file at this URI (${statusText(err)}).`;
    case "mast_bad_input":
    case "mast_uri_refused":
      return sentence(err.message);
    default:
      return errorView(err, { surface: "products" }).text || sentence(err.message);
  }
}

export function footprintBlockText(blocked: string | null, mission: MastMissionId): string | null {
  if (blocked === NO_FOOTPRINT_TEXT && peekableMission(mission)) return `${NO_FOOTPRINT_TEXT}. ${PEEK_HINT}`;
  return blocked;
}
