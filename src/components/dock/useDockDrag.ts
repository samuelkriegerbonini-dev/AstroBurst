import { useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore } from "react";
import { dockStore } from "../../hooks/useDockLayout";
import {
  ANCHOR_LABELS,
  DOCK_TOOL_META,
  anchorOf,
  indexOf,
  moveLabel,
  type DockAnchor,
  type DockLayout,
  type DockToolId,
} from "../../utils/dockLayout";
import {
  dragStarted,
  dropPreviewRect,
  dropTargetAt,
  emptyGroupSlotRect,
  insertionMarkerRect,
  type DockGeometry,
  type DragSource,
  type DropTarget,
  type Point,
  type Rect,
} from "../../utils/dockDrag";
import { focusStripButton, measureDockGeometry, stripShowsFile } from "./dockGeometry";

export interface DockDragState {
  tool: DockToolId;
  from: DockAnchor;
  fromIndex: number;
  pointer: Point;
  target: DropTarget | null;
  preview: Rect | null;
  marker: Rect | null;
  slot: Rect | null;
  label: string | null;
}

export interface StripButtonDragProps {
  onPointerDown(e: React.PointerEvent<HTMLButtonElement>): void;
  onPointerMove(e: React.PointerEvent<HTMLButtonElement>): void;
  onPointerUp(e: React.PointerEvent<HTMLButtonElement>): void;
  onPointerCancel(e: React.PointerEvent<HTMLButtonElement>): void;
  onLostPointerCapture(e: React.PointerEvent<HTMLButtonElement>): void;
  onClickCapture(e: React.MouseEvent<HTMLButtonElement>): void;
}

interface DockDragController {
  buttonProps(tool: DockToolId, anchor: DockAnchor, index: number): StripButtonDragProps;
}

interface ValueStore<T> {
  subscribe(cb: () => void): () => void;
  get(): T;
  set(next: T): void;
}

function createValueStore<T>(initial: T): ValueStore<T> {
  let value = initial;
  const listeners = new Set<() => void>();
  return {
    subscribe(cb) {
      listeners.add(cb);
      return () => {
        listeners.delete(cb);
      };
    },
    get() {
      return value;
    },
    set(next) {
      if (Object.is(next, value)) return;
      value = next;
      listeners.forEach((l) => l());
    },
  };
}

const dragState = createValueStore<DockDragState | null>(null);

export const dragStore: { subscribe(cb: () => void): () => void; get(): DockDragState | null } = {
  subscribe: dragState.subscribe,
  get: dragState.get,
};

export function useDockDragState(): DockDragState | null {
  return useSyncExternalStore(dragStore.subscribe, dragStore.get, () => null);
}

export interface Announcement {
  text: string;
  seq: number;
}

const announcement = createValueStore<Announcement>({ text: "", seq: 0 });

export function announce(text: string): void {
  announcement.set({ text, seq: announcement.get().seq + 1 });
}

export function useAnnouncement(): Announcement {
  return useSyncExternalStore(announcement.subscribe, announcement.get, announcement.get);
}

export function moveAnnouncement(before: DockLayout, after: DockLayout, tool: DockToolId): string | null {
  const label = DOCK_TOOL_META[tool].label;
  const to = anchorOf(after, tool);
  if (anchorOf(before, tool) !== to) return `${label} moved to ${ANCHOR_LABELS[to]}`;
  const delta = indexOf(after, tool) - indexOf(before, tool);
  if (delta < 0) return `${label} moved up`;
  if (delta > 0) return `${label} moved down`;
  return null;
}

interface DragConfig {
  rootRef: React.RefObject<HTMLElement | null>;
  hasFile: boolean | undefined;
}

interface DragSession {
  pointerId: number;
  start: Point;
  pointer: Point;
  source: DragSource;
  geometry: DockGeometry | null;
  hasFile: boolean;
}

function pointOf(e: React.PointerEvent): Point {
  return { x: e.clientX, y: e.clientY };
}

function dragFrame(session: DragSession, geometry: DockGeometry, layout: DockLayout): DockDragState {
  const { source, pointer, hasFile } = session;
  const target = dropTargetAt(pointer, geometry, source);
  const group = target ? geometry.groups.find((g) => g.anchor === target.anchor) : undefined;
  return {
    tool: source.tool,
    from: source.from,
    fromIndex: source.fromIndex,
    pointer,
    target,
    preview: target ? dropPreviewRect(target, geometry, layout, source, hasFile) : null,
    marker: target ? insertionMarkerRect(target, geometry, source) : null,
    slot: target && (group === undefined || group.items.length === 0) ? emptyGroupSlotRect(target.anchor, geometry) : null,
    label: target && target.anchor !== source.from ? moveLabel(target.anchor) : null,
  };
}

function focusToolButton(root: HTMLElement | null, tool: DockToolId): void {
  requestAnimationFrame(() => {
    focusStripButton(root?.querySelector<HTMLElement>(`[data-tool-id="${tool}"]`));
  });
}

function createDockDragController(config: React.RefObject<DragConfig>): DockDragController & { dispose(): void } {
  let session: DragSession | null = null;
  let dragHappened = false;
  let frame = 0;
  let detach: (() => void) | null = null;

  const publish = () => {
    frame = 0;
    if (session?.geometry) dragState.set(dragFrame(session, session.geometry, dockStore.get()));
  };

  const schedule = () => {
    if (frame === 0) frame = requestAnimationFrame(publish);
  };

  const end = () => {
    if (frame !== 0) cancelAnimationFrame(frame);
    frame = 0;
    detach?.();
    detach = null;
    session = null;
    dragState.set(null);
  };

  const begin = (s: DragSession, root: HTMLElement) => {
    s.geometry = measureDockGeometry(root);
    s.hasFile = config.current.hasFile ?? stripShowsFile(root);
    dragHappened = true;
    window.getSelection()?.removeAllRanges();
    const body = document.body.style;
    const cursor = body.cursor;
    const userSelect = body.userSelect;
    body.cursor = "grabbing";
    body.userSelect = "none";
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopImmediatePropagation();
      if (e.type === "keydown" && e.key === "Escape") end();
    };
    const onResize = () => {
      if (!session) return;
      session.geometry = measureDockGeometry(root);
      schedule();
    };
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("keyup", onKey, true);
    window.addEventListener("blur", end);
    window.addEventListener("resize", onResize);
    detach = () => {
      body.cursor = cursor;
      body.userSelect = userSelect;
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("keyup", onKey, true);
      window.removeEventListener("blur", end);
      window.removeEventListener("resize", onResize);
    };
  };

  const drop = (s: DragSession, p: Point) => {
    const target = s.geometry ? dropTargetAt(p, s.geometry, s.source) : null;
    end();
    if (target === null) return;
    const { tool } = s.source;
    const before = dockStore.get();
    dockStore.dispatch({ type: "move", tool, anchor: target.anchor, index: target.index });
    const after = dockStore.get();
    if (after === before) return;
    const message = moveAnnouncement(before, after, tool);
    if (message) announce(message);
    focusToolButton(config.current.rootRef.current, tool);
  };

  const clearDragHappened = () => {
    dragHappened = false;
  };

  const abandon = (e: React.PointerEvent) => {
    if (session === null || session.pointerId !== e.pointerId) return;
    end();
    window.setTimeout(clearDragHappened, 0);
  };

  return {
    buttonProps(tool, anchor, index) {
      return {
        onPointerDown(e) {
          if (e.button !== 0 || !e.isPrimary) return;
          if (session) end();
          dragHappened = false;
          const start = pointOf(e);
          session = { pointerId: e.pointerId, start, pointer: start, source: { tool, from: anchor, fromIndex: index }, geometry: null, hasFile: true };
          e.currentTarget.setPointerCapture(e.pointerId);
        },
        onPointerMove(e) {
          const s = session;
          if (s === null || s.pointerId !== e.pointerId) return;
          s.pointer = pointOf(e);
          if (s.geometry === null) {
            const root = config.current.rootRef.current;
            if (root === null || !dragStarted(s.start, s.pointer)) return;
            begin(s, root);
          }
          schedule();
        },
        onPointerUp(e) {
          if (dragHappened) window.setTimeout(clearDragHappened, 0);
          const s = session;
          if (s === null || s.pointerId !== e.pointerId) return;
          if (s.geometry === null) {
            session = null;
            return;
          }
          drop(s, pointOf(e));
        },
        onPointerCancel: abandon,
        onLostPointerCapture: abandon,
        onClickCapture(e) {
          if (!dragHappened) return;
          dragHappened = false;
          if (e.detail === 0) return;
          e.preventDefault();
          e.stopPropagation();
        },
      };
    },
    dispose() {
      if (session) end();
    },
  };
}

export function useDockDrag(rootRef: React.RefObject<HTMLElement | null>, hasFile?: boolean): DockDragController {
  const config = useRef<DragConfig>({ rootRef, hasFile });
  useLayoutEffect(() => {
    config.current = { rootRef, hasFile };
  });
  const [controller] = useState(() => createDockDragController(config));
  useEffect(() => () => controller.dispose(), [controller]);
  return controller;
}
