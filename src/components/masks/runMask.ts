import { useCallback, useId, useSyncExternalStore } from "react";
import type { MaskChoice } from "../../shared/types/masks";
import type { HistoryScope, NodeMaskView } from "../../shared/types/ops";
import { COMPOSITE_RUN_KEY } from "../../utils/compositeChain";
import { historyRerunStore, type PendingRerun, type RerunSection } from "../../utils/historyRerunStore";
import { maskStore, sameRecipe, type MaskEntry } from "../../utils/maskStore";
import type { MaskSelectProps } from "./MaskSelect";
import { recordedMaskLabel, rerunKeepsMaskReason, unlistedMaskLabel, unlistedMaskReason } from "./maskText";

export interface ResolvedRunMask {
  value: MaskChoice | null;
  selectedId: string | null;
  recorded: string | null;
  unlistedReason: string | null;
}

export interface RunMask {
  value: MaskChoice | null;
  blockedReason: string | null;
  select: Omit<MaskSelectProps, "busy">;
}

const NO_PICK: ResolvedRunMask = { value: null, selectedId: null, recorded: null, unlistedReason: null };

const picks = new Map<string, NodeMaskView>();
const listeners = new Set<() => void>();
let restoredSeq = 0;

function pickKey(section: RerunSection, imageKey: string): string {
  return `${section}|${imageKey}`;
}

export const runMaskStore = {
  get(section: RerunSection, imageKey: string | null): NodeMaskView | null {
    return imageKey === null ? null : (picks.get(pickKey(section, imageKey)) ?? null);
  },
  pick(section: RerunSection, imageKey: string, mask: NodeMaskView | null): void {
    const key = pickKey(section, imageKey);
    if ((picks.get(key) ?? null) === mask) return;
    if (mask === null) picks.delete(key);
    else picks.set(key, mask);
    for (const listener of listeners) listener();
  },
  subscribe(listener: () => void): () => void {
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
    };
  },
};

function restoreRerunMask(pending: PendingRerun | null): void {
  if (!pending || pending.seq === restoredSeq || pending.mask === undefined) return;
  restoredSeq = pending.seq;
  runMaskStore.pick(pending.section, pending.scope.imageKey, pending.mask);
}

restoreRerunMask(historyRerunStore.get());
historyRerunStore.subscribe(() => restoreRerunMask(historyRerunStore.get()));

export function resolveRunMask(masks: readonly MaskEntry[], pick: NodeMaskView | null): ResolvedRunMask {
  if (pick === null) return NO_PICK;
  const { id, recipe, deleted } = pick;
  if (id === null) return recipe ? { value: recipe, selectedId: null, recorded: recordedMaskLabel(recipe), unlistedReason: null } : NO_PICK;
  const listed = masks.find((m) => m.id === id && (recipe === null || sameRecipe(m.recipe, recipe)));
  if (listed) return { value: listed.id, selectedId: listed.id, recorded: null, unlistedReason: null };
  return { value: id, selectedId: null, recorded: unlistedMaskLabel(id, recipe, deleted), unlistedReason: unlistedMaskReason(id, deleted) };
}

function sameScope(a: HistoryScope, b: HistoryScope): boolean {
  return a.imageKey === b.imageKey && a.basePath === b.basePath;
}

export function useRunMask(section: RerunSection, scope: HistoryScope | null, onComposite: boolean): RunMask {
  const imageKey = onComposite ? COMPOSITE_RUN_KEY : (scope?.imageKey ?? null);
  const listedOf = useCallback(() => maskStore.masksOf(imageKey), [imageKey]);
  const masks = useSyncExternalStore(maskStore.subscribe, listedOf, listedOf);
  const pickOf = useCallback(() => runMaskStore.get(section, imageKey), [section, imageKey]);
  const pick = useSyncExternalStore(runMaskStore.subscribe, pickOf, pickOf);
  const armed = useSyncExternalStore(historyRerunStore.subscribe, historyRerunStore.armed, historyRerunStore.armed);
  const noteId = useId();
  const onSelect = useCallback(
    (id: string | null) => {
      if (imageKey === null) return;
      const entry = id === null ? undefined : maskStore.masksOf(imageKey).find((m) => m.id === id);
      runMaskStore.pick(section, imageKey, entry ? { id: entry.id, recipe: entry.recipe, deleted: false } : null);
    },
    [section, imageKey],
  );
  const locked = !onComposite && scope !== null && armed !== null && armed.section === section && sameScope(armed.scope, scope) ? armed : null;
  const resolved = resolveRunMask(masks, pick);
  const blockedReason = locked ? null : resolved.unlistedReason;
  return {
    value: resolved.value,
    blockedReason,
    select: {
      section,
      masks,
      selectedId: resolved.selectedId,
      recorded: resolved.recorded,
      disabledReason: locked ? rerunKeepsMaskReason(locked.node, !!locked.mask) : null,
      note: blockedReason,
      noteId,
      onSelect,
    },
  };
}
