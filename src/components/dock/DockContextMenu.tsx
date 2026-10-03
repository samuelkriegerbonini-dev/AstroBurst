import { useCallback, useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Check } from "lucide-react";
import { dockStore, useDockLayout } from "../../hooks/useDockLayout";
import {
  ANCHOR_LABELS,
  DOCK_ANCHORS,
  DOCK_TOOL_META,
  anchorOf,
  anchorSide,
  indexOf,
  type DockAnchor,
  type DockLayout,
  type DockSide,
  type DockToolId,
} from "../../utils/dockLayout";
import { announce, moveAnnouncement } from "./useDockDrag";
import { focusStripButton } from "./dockGeometry";
import "./dock.css";

export interface DockMenuRequest {
  tool: DockToolId;
  trigger: HTMLButtonElement;
}

interface DockContextMenuProps {
  request: DockMenuRequest | null;
  onClose(): void;
}

interface DockMenuListProps {
  tool: DockToolId;
  anchor: DockAnchor;
  index: number;
  count: number;
  onMove(anchor: DockAnchor): void;
  onReorder(delta: -1 | 1): void;
  onReset(): void;
  onClose(): void;
  id?: string;
}

const ITEM_SELECTOR = '[role="menuitem"], [role="menuitemradio"]';
const MENU_WIDTH_PX = 184;
const MENU_GAP_PX = 6;
const VIEWPORT_MARGIN_PX = 8;

const ITEM_CLASS =
  "w-full flex items-center gap-2 px-3 py-1.5 text-left text-[11px] text-zinc-300 hover:bg-zinc-800 focus:bg-zinc-800 aria-disabled:text-zinc-600 aria-disabled:cursor-default aria-disabled:hover:bg-transparent";

function focusIndex(key: string, current: number, count: number): number | null {
  if (count === 0) return null;
  switch (key) {
    case "ArrowDown":
      return current < 0 ? 0 : (current + 1) % count;
    case "ArrowUp":
      return current < 0 ? count - 1 : (current - 1 + count) % count;
    case "Home":
      return 0;
    case "End":
      return count - 1;
    default:
      return null;
  }
}

export function DockMenuList({ tool, anchor, index, count, onMove, onReorder, onReset, onClose, id }: DockMenuListProps): React.JSX.Element {
  const upDisabled = index <= 0;
  const downDisabled = index >= count - 1;

  const onKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    e.stopPropagation();
    if (e.key === "Escape" || e.key === "Tab") {
      e.preventDefault();
      onClose();
      return;
    }
    const items = Array.from(e.currentTarget.querySelectorAll<HTMLElement>(ITEM_SELECTOR));
    const next = focusIndex(e.key, items.indexOf(e.target as HTMLElement), items.length);
    if (next === null) return;
    e.preventDefault();
    items[next].focus();
  };

  const onBlur = (e: React.FocusEvent<HTMLDivElement>) => {
    if (e.relatedTarget !== null && e.currentTarget.contains(e.relatedTarget)) return;
    onClose();
  };

  return (
    <div
      id={id}
      role="menu"
      data-dock-menu=""
      aria-label={`${DOCK_TOOL_META[tool].label} options`}
      onKeyDown={onKeyDown}
      onKeyUp={(e) => e.stopPropagation()}
      onBlur={onBlur}
      onMouseDown={(e) => e.preventDefault()}
      onContextMenu={(e) => e.preventDefault()}
      className="py-1 bg-zinc-900 border border-zinc-700 rounded-lg shadow-xl"
      style={{ width: MENU_WIDTH_PX }}
    >
      <div aria-hidden="true" className="px-3 pt-1 pb-1 text-[9px] uppercase tracking-wider text-zinc-500">Move to</div>
      <div role="group" aria-label="Move to">
        {DOCK_ANCHORS.map((a) => (
          <button
            key={a}
            type="button"
            role="menuitemradio"
            aria-checked={a === anchor}
            tabIndex={-1}
            onClick={() => (a === anchor ? onClose() : onMove(a))}
            className={ITEM_CLASS}
          >
            <span className="w-3 flex justify-center text-[var(--ab-blue)]">{a === anchor && <Check size={11} aria-hidden="true" />}</span>
            {ANCHOR_LABELS[a]}
          </button>
        ))}
      </div>
      <div role="separator" className="my-1 h-px bg-zinc-800" />
      <button
        type="button"
        role="menuitem"
        aria-disabled={upDisabled || undefined}
        tabIndex={-1}
        onClick={() => {
          if (!upDisabled) onReorder(-1);
        }}
        className={ITEM_CLASS}
      >
        <span className="w-3" />
        Move up
      </button>
      <button
        type="button"
        role="menuitem"
        aria-disabled={downDisabled || undefined}
        tabIndex={-1}
        onClick={() => {
          if (!downDisabled) onReorder(1);
        }}
        className={ITEM_CLASS}
      >
        <span className="w-3" />
        Move down
      </button>
      <div role="separator" className="my-1 h-px bg-zinc-800" />
      <button type="button" role="menuitem" tabIndex={-1} onClick={onReset} className={ITEM_CLASS}>
        <span className="w-3" />
        Reset layout
      </button>
    </div>
  );
}

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(value, max));
}

function menuPosition(trigger: DOMRect, side: DockSide, height: number): { left: number; top: number } {
  const preferred = side === "left" ? trigger.right + MENU_GAP_PX : trigger.left - MENU_GAP_PX - MENU_WIDTH_PX;
  return {
    left: clamp(preferred, VIEWPORT_MARGIN_PX, window.innerWidth - MENU_WIDTH_PX - VIEWPORT_MARGIN_PX),
    top: clamp(trigger.top, VIEWPORT_MARGIN_PX, window.innerHeight - height - VIEWPORT_MARGIN_PX),
  };
}

function returnFocus(tool: DockToolId, trigger: HTMLButtonElement): void {
  const active = document.activeElement;
  if (active !== null && active !== document.body && active.isConnected) return;
  const target = trigger.isConnected ? trigger : document.querySelector<HTMLElement>(`[data-dock-root] [data-tool-id="${tool}"]`);
  focusStripButton(target);
}

function DockMenuPopup({ request, onClose }: { request: DockMenuRequest; onClose(): void }): React.ReactNode {
  const { tool, trigger } = request;
  const layout = useDockLayout();
  const anchor = anchorOf(layout, tool);
  const index = indexOf(layout, tool);
  const count = layout.anchors[anchor].length;
  const side = anchorSide(anchor);
  const menuId = useId();
  const boxRef = useRef<HTMLDivElement>(null);
  const onCloseRef = useRef(onClose);
  const [position, setPosition] = useState<{ left: number; top: number } | null>(null);
  const placed = position !== null;

  useLayoutEffect(() => {
    onCloseRef.current = onClose;
  });

  const close = useCallback(() => {
    requestAnimationFrame(() => returnFocus(tool, trigger));
    onCloseRef.current();
  }, [tool, trigger]);

  useLayoutEffect(() => {
    const box = boxRef.current;
    if (box) setPosition(menuPosition(trigger.getBoundingClientRect(), side, box.offsetHeight));
  }, [trigger, side]);

  useLayoutEffect(() => {
    trigger.setAttribute("aria-expanded", "true");
    trigger.setAttribute("aria-controls", menuId);
    return () => {
      trigger.removeAttribute("aria-expanded");
      trigger.removeAttribute("aria-controls");
    };
  }, [trigger, menuId]);

  useEffect(() => {
    if (placed) boxRef.current?.querySelector<HTMLElement>(ITEM_SELECTOR)?.focus({ preventScroll: true });
  }, [placed]);

  useEffect(() => {
    const inside = (target: EventTarget | null) => target instanceof Node && !!boxRef.current?.contains(target);
    const onPointerDown = (e: PointerEvent) => {
      if (!inside(e.target)) close();
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      e.stopImmediatePropagation();
      close();
    };
    const onScroll = (e: Event) => {
      if (!inside(e.target)) close();
    };
    document.addEventListener("pointerdown", onPointerDown, true);
    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", close);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown, true);
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", close);
    };
  }, [close]);

  const reportMove = (before: DockLayout) => {
    const message = moveAnnouncement(before, dockStore.get(), tool);
    if (message) announce(message);
  };

  const onMove = (target: DockAnchor) => {
    const before = dockStore.get();
    dockStore.dispatch({ type: "move", tool, anchor: target });
    reportMove(before);
    close();
  };

  const onReorder = (delta: -1 | 1) => {
    const before = dockStore.get();
    dockStore.dispatch({ type: "reorder", anchor, from: index, to: index + delta });
    reportMove(before);
    close();
  };

  const onReset = () => {
    dockStore.resetAll();
    announce("Layout reset");
    close();
  };

  return createPortal(
    <div ref={boxRef} className="ab-dock-menu" style={position ?? { left: 0, top: 0, visibility: "hidden" }}>
      <DockMenuList
        id={menuId}
        tool={tool}
        anchor={anchor}
        index={index}
        count={count}
        onMove={onMove}
        onReorder={onReorder}
        onReset={onReset}
        onClose={close}
      />
    </div>,
    document.body,
  );
}

export default function DockContextMenu({ request, onClose }: DockContextMenuProps): React.ReactNode {
  if (request === null) return null;
  return <DockMenuPopup key={request.tool} request={request} onClose={onClose} />;
}
