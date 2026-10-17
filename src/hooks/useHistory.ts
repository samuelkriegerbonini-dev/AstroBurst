import { useCallback, useEffect, useMemo, useRef, useSyncExternalStore } from "react";
import { announce } from "../components/dock/useDockDrag";
import { fileKeyOf, getRenderRecord, useFileContext, useRenderActions, useRenderContext } from "../context/PreviewContext";
import { getPreviewUrl } from "../infrastructure/tauri";
import { deleteBranch, getHistory, getKernel, redo, rehydrate, setHead, undo } from "../services/ops";
import type { HeadView, HistoryNode, HistoryScope, HistoryState, KernelRef } from "../shared/types/ops";
import type { ChainBase, ProcessedResult, ProcessingChain } from "../shared/types/preview";
import { ancestry, headKernelRef, nodeFile, nodePreviewVersion, nodeRecord, opLabel, projectChain, type PreviewLookup } from "../utils/historyModel";
import { EMPTY_HISTORY_SNAPSHOT, historyStore, type HistorySnapshot } from "../utils/historyStore";
import { putCapped, withVersionParam } from "../utils/processingChain";
import { historyRerunStore, sectionForOp } from "../utils/historyRerunStore";
import { dockStore } from "./useDockLayout";
import { runExclusive } from "./useProcessingRun";

export const HISTORY_BUSY_MESSAGE = "A step is still running on this image";
const NO_HISTORY_MESSAGE = "this image has no processing history yet";
const NOTHING_TO_UNDO_MESSAGE = "nothing to undo: the original image is already shown";
const NOTHING_TO_REDO_MESSAGE = "nothing to redo from the current step";
const HISTORY_RUN_PANEL = "history";

export interface HistoryTarget {
  fileKey: string;
  scope: HistoryScope;
  base: ChainBase | null;
  rootPreviewUrl: string | null;
}

export interface HistoryRender {
  currentFileKey: () => string | null;
  publishProcessed: (fileKey: string, result: ProcessedResult, chainUpdate?: (c: ProcessingChain) => ProcessingChain) => void;
  setChain: (fileKey: string, update: (c: ProcessingChain) => ProcessingChain) => void;
  resetProcessed: () => void;
  setHistoryId: (fileKey: string, historyId: string | null) => void;
}

export type HeadMoveRequest = { kind: "undo" } | { kind: "redo" } | { kind: "to"; node: string | null };

function messageOf(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

function sameScope(a: HistoryScope | null, b: HistoryScope): boolean {
  return a !== null && a.imageKey === b.imageKey && a.basePath === b.basePath;
}

function knownState(target: HistoryTarget): HistoryState | null {
  const snap = historyStore.get();
  return snap.fileKey === target.fileKey && sameScope(snap.scope, target.scope) ? snap.state : null;
}

function publishState(target: HistoryTarget, render: HistoryRender, patch: Partial<HistorySnapshot>): void {
  if (render.currentFileKey() !== target.fileKey) return;
  const base: HistorySnapshot = { ...EMPTY_HISTORY_SNAPSHOT, state: knownState(target) };
  historyStore.set({ ...base, ...patch, fileKey: target.fileKey, scope: target.scope });
}

const historyQueues = new Map<string, Promise<void>>();

function queued<T>(fileKey: string, task: () => Promise<T>): Promise<T> {
  const previous = historyQueues.get(fileKey) ?? Promise.resolve();
  const next = previous.then(task);
  const settled = next.then(
    () => undefined,
    () => undefined,
  );
  historyQueues.set(fileKey, settled);
  void settled.then(() => {
    if (historyQueues.get(fileKey) === settled) historyQueues.delete(fileKey);
  });
  return next;
}

function exclusive<T>(fileKey: string, task: () => Promise<T>) {
  return queued(fileKey, () => runExclusive(HISTORY_RUN_PANEL, fileKey, task));
}

const KERNEL_CACHE_MAX = 16;
const kernelCache = new Map<string, number[][]>();

async function kernelsFor(target: HistoryTarget, imageId: string, ref: KernelRef | null): Promise<Map<string, number[][]> | undefined> {
  if (!ref) return undefined;
  const cacheKey = `${imageId}:${ref.hash}`;
  let rows = kernelCache.get(cacheKey);
  if (!rows) {
    rows = await getKernel(target.scope, ref);
    putCapped(kernelCache, cacheKey, rows, KERNEL_CACHE_MAX);
  }
  return new Map([[ref.hash, rows]]);
}

async function previewsFor(state: HistoryState, head: string | null): Promise<PreviewLookup> {
  const doc = state.doc;
  const nodes: HistoryNode[] = doc && head !== null ? ancestry(doc, head) : [];
  const pngs = nodes.map((n) => nodeFile(state.history_dir, n.output.png));
  const urls = await Promise.all(
    nodes.map(async (n, i) => {
      const url = await getPreviewUrl(pngs[i]);
      const version = nodePreviewVersion(n);
      return url && version !== null ? withVersionParam(url, version) : url;
    }),
  );
  const byPng = new Map(pngs.map((png, i) => [png, urls[i]]));
  return (png) => byPng.get(png) ?? null;
}

function baseRecord(base: ChainBase): ProcessedResult {
  return { fitsPath: base.path, previewUrl: base.previewUrl, dimensions: base.dimensions, label: base.label, kind: base.kind, inputPath: base.inputPath };
}

export async function showHead(target: HistoryTarget, render: HistoryRender, state: HistoryState, head: string | null): Promise<void> {
  const doc = state.doc;
  if (!doc) return;
  const previewOf = await previewsFor(state, head);
  const kernels = await kernelsFor(target, state.image_id, headKernelRef(doc, head));
  const psfKernel = getRenderRecord(target.fileKey)?.chain.psfKernel ?? null;
  const dir = state.history_dir;
  const chain = projectChain(doc, head, { dir, base: target.base, psfKernel, kernels, previewOf });
  if (head !== null) {
    const record = nodeRecord(doc, head, { dir, rootPath: state.root_path, rootPreviewUrl: target.rootPreviewUrl, previewOf });
    if (record) render.publishProcessed(target.fileKey, record, () => chain);
  } else if (target.base) {
    render.publishProcessed(target.fileKey, baseRecord(target.base), () => chain);
  } else if (render.currentFileKey() === target.fileKey) {
    render.resetProcessed();
    const kernel = chain.psfKernel;
    if (kernel) render.setChain(target.fileKey, (c) => ({ ...c, psfKernel: kernel }));
  }
  render.setHistoryId(target.fileKey, state.image_id);
}

function headCall(scope: HistoryScope, request: HeadMoveRequest): Promise<HeadView> {
  if (request.kind === "undo") return undo(scope);
  if (request.kind === "redo") return redo(scope);
  return setHead(scope, request.node);
}

function nodeLabel(state: HistoryState | null, id: string | null | undefined): string | null {
  const node = id ? state?.doc?.nodes.find((n) => n.id === id) : undefined;
  return node ? opLabel(node.op) : null;
}

function moveMessage(request: HeadMoveRequest, before: HistoryState | null, view: HeadView): string {
  if (request.kind === "undo") {
    const label = nodeLabel(before, before?.doc?.head);
    return label ? `Undo: ${label}` : "Undo";
  }
  const shown = view.node ? opLabel(view.node.op) : "the original image";
  return request.kind === "redo" ? `Redo: ${shown}` : `Showing ${shown}`;
}

async function settledState(scope: HistoryScope, view: HeadView): Promise<HistoryState> {
  if (view.node && !view.node.present) await rehydrate(scope, view.node.id);
  return getHistory(scope);
}

async function restoreHead(scope: HistoryScope, previousHead: string | null | undefined): Promise<void> {
  if (previousHead !== undefined) await setHead(scope, previousHead).catch(() => undefined);
}

export function moveHead(target: HistoryTarget, render: HistoryRender, request: HeadMoveRequest): Promise<HeadView | null> {
  return queued(target.fileKey, () => moveHeadNow(target, render, request));
}

function idleMoveMessage(request: HeadMoveRequest, state: HistoryState | null): string | null {
  if (!state || state.read_only !== null || request.kind === "to") return null;
  const doc = state.doc;
  if (!doc || doc.nodes.length === 0) return NO_HISTORY_MESSAGE;
  if (request.kind === "undo") return doc.head === null ? NOTHING_TO_UNDO_MESSAGE : null;
  return doc.nodes.some((n) => n.parent === doc.head) ? null : NOTHING_TO_REDO_MESSAGE;
}

async function moveHeadNow(target: HistoryTarget, render: HistoryRender, request: HeadMoveRequest): Promise<HeadView | null> {
  const before = knownState(target);
  const idle = idleMoveMessage(request, before);
  try {
    const outcome = await runExclusive(HISTORY_RUN_PANEL, target.fileKey, async (): Promise<HeadView | string> => {
      if (idle !== null) return idle;
      const view = await headCall(target.scope, request);
      try {
        const state = await settledState(target.scope, view);
        await showHead(target, render, state, view.head);
        publishState(target, render, { state, error: null });
      } catch (e) {
        await restoreHead(target.scope, before?.doc?.head);
        throw e;
      }
      return view;
    });
    if (!outcome.ran) {
      announce(HISTORY_BUSY_MESSAGE);
      return null;
    }
    if (typeof outcome.value === "string") {
      announce(outcome.value);
      return null;
    }
    announce(moveMessage(request, before, outcome.value));
    return outcome.value;
  } catch (e) {
    const message = messageOf(e);
    announce(message);
    publishState(target, render, { state: before, error: message });
    return null;
  }
}

export async function loadHistory(target: HistoryTarget, render: HistoryRender, wanted: () => boolean = () => true): Promise<HistoryState | null> {
  publishState(target, render, { pending: true, error: null });
  try {
    const state = await getHistory(target.scope);
    if (!wanted()) return null;
    publishState(target, render, { state, error: null });
    const doc = state.doc;
    if (state.read_only !== null || !doc) return state;
    if (getRenderRecord(target.fileKey)?.historyId === state.image_id) return state;
    if (doc.head === null) {
      render.setHistoryId(target.fileKey, state.image_id);
      return state;
    }
    if (state.fingerprint_matches === false) {
      await exclusive(target.fileKey, async () => {
        await setHead(target.scope, null);
        const reverted = await getHistory(target.scope);
        if (!wanted()) return;
        render.setHistoryId(target.fileKey, state.image_id);
        publishState(target, render, { state: reverted, error: null });
      });
      return state;
    }
    const head = doc.head;
    await exclusive(target.fileKey, async () => {
      const present = doc.nodes.find((n) => n.id === head)?.present ?? false;
      const settled = present ? state : await rehydrate(target.scope, head).then(() => getHistory(target.scope));
      if (!wanted()) return;
      await showHead(target, render, settled, head);
      publishState(target, render, { state: settled, error: null });
    });
    return state;
  } catch (e) {
    if (wanted()) publishState(target, render, { error: messageOf(e) });
    return null;
  }
}

export async function adoptRun(target: HistoryTarget, render: HistoryRender, node?: string): Promise<void> {
  const state = await getHistory(target.scope);
  publishState(target, render, { state, error: null });
  const doc = state.doc;
  if (!doc || (node !== undefined && !doc.nodes.some((n) => n.id === node))) return;
  const record = getRenderRecord(target.fileKey);
  if (record && (record.chain.base?.path ?? null) !== (target.base?.path ?? null)) return;
  await showHead(target, render, state, doc.head);
}

export async function revertToOriginal(target: HistoryTarget, render: HistoryRender): Promise<void> {
  const known = knownState(target);
  const recorded = known?.read_only === null && (known.doc?.nodes.length ?? 0) > 0;
  if (!recorded || known.doc?.head === null) {
    render.resetProcessed();
    return;
  }
  await moveHead(target, render, { kind: "to", node: null });
}

export async function requestRerun(target: HistoryTarget, render: HistoryRender, node: HistoryNode, downstream: boolean): Promise<boolean> {
  const section = sectionForOp(node.op);
  if (!section) return false;
  if (!downstream && !(await moveHead(target, render, { kind: "to", node: node.id }))) return false;
  historyRerunStore.request({ section, op: node.op, node: node.id, downstream, scope: target.scope });
  dockStore.dispatch({ type: "open", tool: "processing" });
  return true;
}

export async function removeBranch(target: HistoryTarget, render: HistoryRender, node: string): Promise<boolean> {
  try {
    const outcome = await exclusive(target.fileKey, async () => {
      const view = await deleteBranch(target.scope, node);
      const state = await getHistory(target.scope);
      await showHead(target, render, state, view.head);
      publishState(target, render, { state, error: null });
      return view.removed.length;
    });
    if (!outcome.ran) {
      announce(HISTORY_BUSY_MESSAGE);
      return false;
    }
    announce(outcome.value === 1 ? "Deleted 1 step" : `Deleted ${outcome.value} steps`);
    return true;
  } catch (e) {
    const message = messageOf(e);
    announce(message);
    publishState(target, render, { error: message });
    return false;
  }
}

export function useHistoryTarget(): HistoryTarget | null {
  const { file } = useFileContext();
  const { chain } = useRenderContext();
  const fileKey = fileKeyOf(file);
  const path = file?.path ?? null;
  const base = chain.base;
  const rootPreviewUrl = base?.previewUrl ?? file?.result?.previewUrl ?? null;
  return useMemo(
    () => (fileKey && path ? { fileKey, scope: { imageKey: path, basePath: base?.path ?? null }, base, rootPreviewUrl } : null),
    [fileKey, path, base, rootPreviewUrl],
  );
}

export function useHistoryRender(): HistoryRender {
  const { currentFileKey, publishProcessed, setChain, resetProcessed, setHistoryId } = useRenderActions();
  return useMemo(
    () => ({ currentFileKey, publishProcessed, setChain, resetProcessed, setHistoryId }),
    [currentFileKey, publishProcessed, setChain, resetProcessed, setHistoryId],
  );
}

export function useHistorySnapshot(): HistorySnapshot {
  return useSyncExternalStore(historyStore.subscribe, historyStore.get, historyStore.get);
}

export interface HistoryController {
  target: HistoryTarget | null;
  undo: () => Promise<void>;
  redo: () => Promise<void>;
  goTo: (node: string | null) => Promise<void>;
  revert: () => Promise<void>;
  deleteBranch: (node: string) => Promise<void>;
  rerunWith: (node: HistoryNode, downstream: boolean) => Promise<void>;
}

export function useHistoryController(): HistoryController {
  const target = useHistoryTarget();
  const render = useHistoryRender();
  const targetRef = useRef(target);
  targetRef.current = target;

  const move = useCallback(
    async (request: HeadMoveRequest) => {
      const current = targetRef.current;
      if (!current) return;
      await moveHead(current, render, request);
    },
    [render],
  );

  const rerunWith = useCallback(
    async (node: HistoryNode, downstream: boolean) => {
      const current = targetRef.current;
      if (current) await requestRerun(current, render, node, downstream);
    },
    [render],
  );

  const revert = useCallback(async () => {
    const current = targetRef.current;
    if (current) await revertToOriginal(current, render);
  }, [render]);

  const removeNode = useCallback(
    async (node: string) => {
      const current = targetRef.current;
      if (current) await removeBranch(current, render, node);
    },
    [render],
  );

  return useMemo(
    () => ({
      target,
      undo: () => move({ kind: "undo" }),
      redo: () => move({ kind: "redo" }),
      goTo: (node: string | null) => move({ kind: "to", node }),
      revert,
      deleteBranch: removeNode,
      rerunWith,
    }),
    [target, move, revert, removeNode, rerunWith],
  );
}

export function useHistory(): HistoryController {
  const controller = useHistoryController();
  const render = useHistoryRender();
  const target = controller.target;
  const targetRef = useRef(target);
  targetRef.current = target;
  const loadKey = target ? `${target.fileKey}\n${target.scope.basePath ?? ""}` : null;

  useEffect(() => {
    const current = targetRef.current;
    if (loadKey === null || !current) {
      historyStore.set(EMPTY_HISTORY_SNAPSHOT);
      return;
    }
    let live = true;
    void loadHistory(current, render, () => live);
    return () => {
      live = false;
    };
  }, [loadKey, render]);

  const hasTarget = target !== null;
  const { undo: undoAction, redo: redoAction } = controller;
  useEffect(() => {
    if (!hasTarget) return;
    return historyStore.bind({ undo: undoAction, redo: redoAction });
  }, [hasTarget, undoAction, redoAction]);

  return controller;
}

export function useHistoryScope(): HistoryScope | null {
  return useHistoryTarget()?.scope ?? null;
}
