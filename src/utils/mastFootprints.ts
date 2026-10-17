import type { MastFootprint, MastFootprintInput, MastRow } from "../shared/types/mast";
import type { RegionWire } from "../shared/types/regions";

export const MAX_FOOTPRINT_VERTICES = 20_000;
export const CIRCLE_VERTICES = 64;
export const MOVING_TEXT = "Moving target: the archive footprint is not sidereal";
export const NO_FOOTPRINT_TEXT = "MAST lists no footprint for this dataset";

const MOVING_TARGTYPE = "MOVING";
const BOX_EXTRA_VERTICES = 2;
const NUMBER = /[-+]?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?/g;
const CIRCLE = /\bcircle\b/gi;
const BOX = /\bbox\b/gi;
const ARCMIN_PER_DEG = 60;

export function isMovingTarget(targtype: string | null): boolean {
  return targtype !== null && targtype.trim().toUpperCase() === MOVING_TARGTYPE;
}

function targtypeOf(row: MastRow): string | null {
  const value = row.values.targtype;
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

export function footprintInput(row: MastRow, polygons: [number, number][][] | null = null): MastFootprintInput {
  const sRegion = row.sRegion?.trim() ? row.sRegion : null;
  return {
    datasetId: row.datasetId,
    sRegion,
    polygons: sRegion === null && polygons?.length ? polygons : null,
    targtype: targtypeOf(row),
  };
}

export function footprintKey(input: MastFootprintInput): string {
  return [input.datasetId, input.sRegion ?? "", input.polygons ? JSON.stringify(input.polygons) : ""].join("\n");
}

export function footprintBlock(input: MastFootprintInput): string | null {
  if (isMovingTarget(input.targtype)) return MOVING_TEXT;
  if (!input.polygons?.length && !input.sRegion?.trim()) return NO_FOOTPRINT_TEXT;
  return null;
}

function count(text: string, pattern: RegExp): number {
  return text.match(pattern)?.length ?? 0;
}

export function estimateVertices(input: MastFootprintInput): number {
  if (input.polygons?.length) return input.polygons.reduce((sum, polygon) => sum + polygon.length, 0);
  const text = input.sRegion ?? "";
  return Math.ceil(count(text, NUMBER) / 2) + count(text, CIRCLE) * CIRCLE_VERTICES + count(text, BOX) * BOX_EXTRA_VERTICES;
}

export function footprintChunks(inputs: readonly MastFootprintInput[], cap: number = MAX_FOOTPRINT_VERTICES): MastFootprintInput[][] {
  const chunks: MastFootprintInput[][] = [];
  let current: MastFootprintInput[] = [];
  let used = 0;
  for (const input of inputs) {
    const vertices = footprintBlock(input) ? 0 : estimateVertices(input);
    if (current.length > 0 && used + vertices > cap) {
      chunks.push(current);
      current = [];
      used = 0;
    }
    current.push(input);
    used += vertices;
  }
  if (current.length > 0) chunks.push(current);
  return chunks;
}

export function polygonCount(footprints: readonly MastFootprint[]): number {
  return footprints.reduce((sum, footprint) => sum + footprint.polygons.length, 0);
}

export function footprintRegions(footprints: readonly MastFootprint[]): RegionWire[] {
  return footprints.flatMap((footprint) =>
    footprint.polygons.map(
      (polygon): RegionWire => ({
        shape: { shape: "polygon", points: polygon.map(([x, y]): [number, number] => [x, y]) },
        props: { color: null, width: null, text: footprint.datasetId, dash: null, include: true },
      }),
    ),
  );
}

export function footprintSummary(footprint: MastFootprint, fileName: string): string {
  const n = footprint.polygons.length;
  if (n === 0) return footprint.note ?? NO_FOOTPRINT_TEXT;
  const parts = [`${n} ${n === 1 ? "polygon" : "polygons"}`, `${footprint.vertices} vertices`];
  const cone = footprint.cone;
  if (cone) parts.push(`cone ${cone.ra.toFixed(4)} ${cone.dec.toFixed(4)} r ${(cone.radiusDeg * ARCMIN_PER_DEG).toFixed(2)}'`);
  parts.push(footprint.onImage ? `overlaps ${fileName}` : `off ${fileName}`);
  const text = parts.join(", ");
  return footprint.note ? `${text}; ${footprint.note}` : text;
}
