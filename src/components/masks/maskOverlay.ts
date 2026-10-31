import { previewMask, type MaskTarget } from "../../services/masks";
import type { MaskPreview, MaskStyle } from "../../shared/types/masks";
import { nodeFile } from "../../utils/historyModel";
import { historyStore, type HistorySnapshot } from "../../utils/historyStore";
import { maskStore, type MaskEntry } from "../../utils/maskStore";
import { overlayStore } from "../../utils/overlayStore";
import { putCapped } from "../../utils/processingChain";
import { MASK_LAYER_ID, maskLayer, type MaskSurface } from "../viewer/painters/maskPainter";

const PREVIEW_CACHE_MAX = 4;

export interface MaskLayerRequest {
  key: string;
  target: MaskTarget;
  entry: MaskEntry;
  style: MaskStyle;
  dims: readonly [number, number];
  revision: string;
}

interface DrawnMask {
  maskId: string;
  revision: string;
}

const previews = new Map<string, Promise<MaskPreview>>();
const drawn = new Map<string, DrawnMask>();
const latestRequest = new Map<string, number>();
let requestSeq = 0;

export function maskInputPath(snapshot: HistorySnapshot): string | null {
  const state = snapshot.state;
  const doc = state && state.read_only === null ? state.doc : null;
  const head = doc && doc.head !== null ? doc.nodes.find((n) => n.id === doc.head) : undefined;
  return state && head ? nodeFile(state.history_dir, head.output.path) : null;
}

function previewKey({ key, revision, entry }: MaskLayerRequest): string {
  return [key, revision, entry.id, JSON.stringify(entry.recipe)].join("\n");
}

function cachedPreview(request: MaskLayerRequest): Promise<MaskPreview> {
  const cacheKey = previewKey(request);
  const known = previews.get(cacheKey);
  if (known) return known;
  const loading = previewMask(request.target, request.entry.id);
  putCapped(previews, cacheKey, loading, PREVIEW_CACHE_MAX);
  loading.catch(() => {
    if (previews.get(cacheKey) === loading) previews.delete(cacheKey);
  });
  return loading;
}

function issue(key: string): number {
  requestSeq += 1;
  latestRequest.set(key, requestSeq);
  return requestSeq;
}

export function hideMaskLayer(key: string): void {
  issue(key);
  drawn.delete(key);
  overlayStore.remove(key, MASK_LAYER_ID);
}

export async function showMaskLayer(request: MaskLayerRequest, surface?: MaskSurface): Promise<void> {
  const token = issue(request.key);
  const preview = await cachedPreview(request);
  if (latestRequest.get(request.key) !== token || maskStore.activeOf(request.key) !== request.entry.id) return;
  const layer = maskLayer({ maskId: request.entry.id, preview, dims: request.dims, style: request.style }, surface);
  if (!layer) {
    hideMaskLayer(request.key);
    return;
  }
  overlayStore.add(request.key, layer);
  drawn.set(request.key, { maskId: request.entry.id, revision: request.revision });
}

export function forgetMaskPreviews(key: string, maskId: string): void {
  for (const cacheKey of [...previews.keys()]) {
    const [ownerKey, , id] = cacheKey.split("\n");
    if (ownerKey === key && id === maskId) previews.delete(cacheKey);
  }
}

maskStore.subscribe(() => {
  for (const [key, mask] of [...drawn]) {
    if (maskStore.activeOf(key) !== mask.maskId) hideMaskLayer(key);
  }
});

historyStore.subscribe(() => {
  const snapshot = historyStore.get();
  const key = snapshot.scope?.imageKey;
  const mask = key ? drawn.get(key) : undefined;
  if (!key || !mask || !snapshot.state) return;
  if (mask.revision !== (maskInputPath(snapshot) ?? "")) hideMaskLayer(key);
});
