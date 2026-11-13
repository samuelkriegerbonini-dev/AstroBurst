import { ANALYSIS_SECTION } from "./analysisSections";
import { FILE_STATUS } from "./constants";
import type { DockToolId } from "./dockLayout";
import type { TableRouteKind } from "./fileKinds";

export interface TableRouteTarget {
  tool: DockToolId;
  sectionId: string;
  needsFile: string;
}

export const TABLE_ROUTE_TARGET: Readonly<Record<TableRouteKind, TableRouteTarget>> = {
  light_curve: {
    tool: "photometry",
    sectionId: ANALYSIS_SECTION.series.id,
    needsFile: "Kept for the Series section of Photometry: select an image or a spectrum, then open Photometry to see the light curve",
  },
  catalogue: {
    tool: "photometry",
    sectionId: ANALYSIS_SECTION.compare.id,
    needsFile: "Kept for the Compare section of Photometry: select the image to compare, then open Photometry to use the catalogue",
  },
};

const ROUTE_HOST_STATUSES: Readonly<Record<TableRouteKind, readonly string[]>> = {
  light_curve: [FILE_STATUS.DONE, FILE_STATUS.SPECTRUM],
  catalogue: [FILE_STATUS.DONE],
};

export function tableRouteHost(kind: TableRouteKind, lastShown: { id: string; status: string } | null | undefined): string | null {
  return lastShown && ROUTE_HOST_STATUSES[kind].includes(lastShown.status) ? lastShown.id : null;
}

export interface TableRouteDeps {
  openTool(tool: DockToolId): void;
  scrollTo(sectionId: string): void;
}

export function followTableRoute(kind: TableRouteKind, fileShown: boolean, deps: TableRouteDeps): string | null {
  const target = TABLE_ROUTE_TARGET[kind];
  if (!fileShown) return target.needsFile;
  deps.openTool(target.tool);
  deps.scrollTo(target.sectionId);
  return null;
}

export interface ScrollTarget {
  scrollIntoView(options?: ScrollIntoViewOptions): void;
}

export const SECTION_MOUNT_TRIES = 30;
export const SECTION_HOLD_MS = 5000;
const USER_SCROLL_EVENTS = ["wheel", "pointerdown", "keydown", "touchstart"] as const;

export function scrollToSectionWhenMounted<T extends ScrollTarget>(
  sectionId: string,
  find: (id: string) => T | null,
  nextFrame: (run: () => void) => void,
  tries = SECTION_MOUNT_TRIES,
  onScrolled?: (target: T) => void,
): void {
  nextFrame(() => {
    const el = find(sectionId);
    if (el) {
      el.scrollIntoView({ block: "start" });
      onScrolled?.(el);
    } else if (tries > 1) scrollToSectionWhenMounted(sectionId, find, nextFrame, tries - 1, onScrolled);
  });
}

export interface SectionHold {
  onResize(run: () => void): () => void;
  onUserInput(run: () => void): () => void;
  after(ms: number, run: () => void): () => void;
}

export function holdSectionAtStart(target: ScrollTarget, hold: SectionHold, ms = SECTION_HOLD_MS): () => void {
  const stops: (() => void)[] = [];
  const stop = () => {
    for (const release of stops.splice(0)) release();
  };
  stops.push(hold.onResize(() => target.scrollIntoView({ block: "start" })), hold.onUserInput(stop), hold.after(ms, stop));
  return stop;
}

export function domSectionHold(section: Element): SectionHold {
  return {
    onResize(run) {
      const observer = new ResizeObserver(() => run());
      for (const el of [section.parentElement, section.closest("[data-dock-host]")]) if (el) observer.observe(el);
      return () => observer.disconnect();
    },
    onUserInput(run) {
      for (const type of USER_SCROLL_EVENTS) window.addEventListener(type, run, { capture: true, passive: true });
      return () => {
        for (const type of USER_SCROLL_EVENTS) window.removeEventListener(type, run, { capture: true });
      };
    },
    after(ms, run) {
      const id = window.setTimeout(run, ms);
      return () => window.clearTimeout(id);
    },
  };
}
