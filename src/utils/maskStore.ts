import type { HistoryDoc, HistoryState } from "../shared/types/ops";
import type { MarkMode, MaskCreateResult, MaskDeleteResult, MaskExportResult, MaskKind, MaskSpec, MaskStyle, MaskTint } from "../shared/types/masks";
import { historyStore, type HistorySnapshot } from "./historyStore";

export const MASK_OPACITY_DEFAULT = 0.45;
export const MASK_TINT_DEFAULT: MaskTint = [255, 0, 0];
export const MASK_MARK_DEFAULT: MarkMode = "nonzero";

const MASK_KINDS: readonly MaskKind[] = ["luminance", "range", "stars", "regions", "file"];

export interface MaskEntry {
  id: string;
  kind: MaskKind;
  recipe: MaskSpec;
  coverage: number | null;
  stars: number | null;
  dims: [number, number] | null;
}

export interface MaskState {
  lists: Readonly<Record<string, readonly MaskEntry[]>>;
  active: Readonly<Record<string, string>>;
  style: MaskStyle;
  mark: MarkMode;
}

export interface MaskHistorySource {
  get: () => HistorySnapshot;
  subscribe: (listener: () => void) => () => void;
}

export type MaskListDoc = Pick<HistoryDoc, "masks" | "deleted_masks">;

const NO_MASKS: readonly MaskEntry[] = Object.freeze([]);

const INITIAL_STATE: MaskState = Object.freeze({
  lists: {},
  active: {},
  style: { opacity: MASK_OPACITY_DEFAULT, tint: MASK_TINT_DEFAULT, displayInvert: false },
  mark: MASK_MARK_DEFAULT,
});

function isMaskSpec(value: unknown): value is MaskSpec {
  return typeof value === "object" && value !== null && MASK_KINDS.includes((value as { kind?: unknown }).kind as MaskKind);
}

function byMaskId(a: MaskEntry, b: MaskEntry): number {
  return a.id.localeCompare(b.id, undefined, { numeric: true });
}

export function sameRecipe(a: MaskSpec, b: MaskSpec): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

function sameTint(a: MaskTint, b: MaskTint): boolean {
  return a[0] === b[0] && a[1] === b[1] && a[2] === b[2];
}

export function listedMasks(doc: MaskListDoc | null): MaskEntry[] {
  if (!doc) return [];
  const deleted = new Set(doc.deleted_masks ?? []);
  return Object.entries(doc.masks)
    .filter((entry): entry is [string, MaskSpec] => !deleted.has(entry[0]) && isMaskSpec(entry[1]))
    .map(([id, recipe]) => ({ id, kind: recipe.kind, recipe, coverage: null, stars: null, dims: null }))
    .sort(byMaskId);
}

export class MaskStoreCore {
  private state: MaskState = INITIAL_STATE;
  private readonly listeners = new Set<() => void>();
  private readonly syncedStates = new WeakSet<HistoryState>();
  private readonly exported: string[] = [];

  constructor(history?: MaskHistorySource) {
    if (!history) return;
    history.subscribe(() => this.syncHistory(history.get()));
    this.syncHistory(history.get());
  }

  get = (): MaskState => this.state;

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  masksOf = (imageKey: string | null): readonly MaskEntry[] => (imageKey === null ? NO_MASKS : (this.state.lists[imageKey] ?? NO_MASKS));

  activeOf = (imageKey: string | null): string | null => (imageKey === null ? null : (this.state.active[imageKey] ?? null));

  syncHistory(snapshot: HistorySnapshot): void {
    const imageKey = snapshot.scope?.imageKey;
    const state = snapshot.state;
    if (!imageKey || !state || this.syncedStates.has(state)) return;
    this.syncedStates.add(state);
    this.syncDoc(imageKey, state.doc);
  }

  syncDoc(imageKey: string, doc: MaskListDoc | null): void {
    const known = this.masksOf(imageKey);
    const next = listedMasks(doc).map((entry) => {
      const seen = known.find((k) => k.id === entry.id);
      return seen && sameRecipe(seen.recipe, entry.recipe) ? seen : entry;
    });
    this.setList(imageKey, next);
  }

  recordCreated(imageKey: string, result: MaskCreateResult): void {
    const entry: MaskEntry = {
      id: result.mask_id,
      kind: result.kind,
      recipe: result.recipe,
      coverage: result.coverage,
      stars: result.stars,
      dims: result.dims,
    };
    const others = this.masksOf(imageKey).filter((e) => e.id !== entry.id);
    this.setList(imageKey, [...others, entry].sort(byMaskId));
  }

  recordDeleted(imageKey: string, result: MaskDeleteResult): void {
    this.setList(
      imageKey,
      this.masksOf(imageKey).filter((e) => e.id !== result.mask_id),
    );
  }

  setActive(imageKey: string, maskId: string | null): void {
    if (maskId === this.activeOf(imageKey)) return;
    if (maskId !== null && !this.masksOf(imageKey).some((e) => e.id === maskId)) return;
    const active = { ...this.state.active };
    if (maskId === null) delete active[imageKey];
    else active[imageKey] = maskId;
    this.commit({ ...this.state, active });
  }

  setStyle(patch: Partial<MaskStyle>): void {
    const current = this.state.style;
    const opacity = patch.opacity !== undefined && Number.isFinite(patch.opacity) ? Math.min(1, Math.max(0, patch.opacity)) : current.opacity;
    const tint = patch.tint ?? current.tint;
    const displayInvert = patch.displayInvert ?? current.displayInvert;
    if (opacity === current.opacity && displayInvert === current.displayInvert && sameTint(tint, current.tint)) return;
    this.commit({ ...this.state, style: { opacity, tint, displayInvert } });
  }

  setMark(mark: MarkMode): void {
    if (mark === this.state.mark) return;
    this.commit({ ...this.state, mark });
  }

  recordExported(result: MaskExportResult): void {
    if (!this.exported.includes(result.fits_path)) this.exported.push(result.fits_path);
  }

  exportedPaths = (): readonly string[] => [...this.exported];

  private setList(imageKey: string, next: readonly MaskEntry[]): void {
    const known = this.masksOf(imageKey);
    const activeId = this.activeOf(imageKey);
    const activeEntry = activeId === null ? undefined : known.find((e) => e.id === activeId);
    const keepsActive =
      activeId === null || (activeEntry !== undefined && next.some((e) => e.id === activeEntry.id && sameRecipe(e.recipe, activeEntry.recipe)));
    const unchanged = next.length === known.length && next.every((entry, i) => entry === known[i]);
    if (unchanged && keepsActive) return;
    const lists = { ...this.state.lists, [imageKey]: next };
    if (keepsActive) {
      this.commit({ ...this.state, lists });
      return;
    }
    const active = { ...this.state.active };
    delete active[imageKey];
    this.commit({ ...this.state, lists, active });
  }

  private commit(next: MaskState): void {
    this.state = next;
    for (const listener of this.listeners) listener();
  }
}

export const maskStore = new MaskStoreCore(historyStore);
