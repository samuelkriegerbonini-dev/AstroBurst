import { anchorOf, moveLabel, type DockAnchor, type DockLayout, type DockSide, type DockToolId } from "./dockLayout";

export const DOCK_ONBOARDING_KEY = "ab.onboarding.dock.v1";
export const DOCK_TOGGLE_TIP_KEY = "ab.onboarding.dockToggle.v1";
export const DOCK_SETTINGS_TIP_KEY = "ab.onboarding.settings.v1";
export const DOCK_ONBOARDING_DONE = "done";
export const DOCK_ONBOARDING_DELAY_MS = 600;
export const DOCK_ONBOARDING_TITLE = "Arrange your tool windows";
export const DOCK_ONBOARDING_TIP = "Right-click a tool (or Shift+F10) for Move to…, and Reset Layout puts everything back.";

export const ONBOARDING_INSET_PX = 16;
export const ONBOARDING_MAX_WIDTH_PX = 360;
export const ONBOARDING_NARROW_VIEWER_PX = 420;
export const ONBOARDING_MIN_HEIGHT_PX = 120;
export const TIP_MAX_WIDTH_PX = 280;
export const TIP_GAP_PX = 10;
export const TIP_ARROW_EDGE_PX = 14;

export type DockTipId = "toggle" | "settings";
export type OnboardingCardId = DockTipId | "tour";

export interface DockTip {
  id: DockTipId;
  tool: DockToolId;
  title: string;
  body: string;
}

export const DOCK_TIPS: Readonly<Record<DockTipId, DockTip>> = {
  toggle: {
    id: "toggle",
    tool: "compose",
    title: "Open and close tools",
    body: "Click any tool in this bar to open it, and click it again to close it.",
  },
  settings: {
    id: "settings",
    tool: "config",
    title: "Settings",
    body: "Click Config for the settings: astrometry.net API key, plate solving, catalogs, archive downloads and output storage. Click it again to close them.",
  },
};

export const DOCK_TIP_IDS: readonly DockTipId[] = ["toggle", "settings"];
export const ONBOARDING_ORDER: readonly OnboardingCardId[] = ["toggle", "settings", "tour"];

export const ONBOARDING_KEYS: Readonly<Record<OnboardingCardId, string>> = {
  toggle: DOCK_TOGGLE_TIP_KEY,
  settings: DOCK_SETTINGS_TIP_KEY,
  tour: DOCK_ONBOARDING_KEY,
};

export function nextOnboardingCard(done: ReadonlySet<OnboardingCardId>, ready: Readonly<Record<OnboardingCardId, boolean>>): OnboardingCardId | null {
  return ONBOARDING_ORDER.find((card) => !done.has(card) && ready[card]) ?? null;
}

export function stripButtonHint(label: string, { open, disabled }: { open: boolean; disabled: boolean }): string {
  if (disabled) return `${label}: open a file first`;
  return open ? `${label} is open: click to close` : `${label}: click to open, click again to close`;
}

export type DockOnboardingDemo = "vertical" | "horizontal";

export interface DockOnboardingStep {
  demo: DockOnboardingDemo;
  title: string;
  body: string;
  anchor: DockAnchor;
  label: string;
}

export const DOCK_ONBOARDING_STEPS: readonly DockOnboardingStep[] = [
  {
    demo: "vertical",
    title: "Drag vertically",
    body: "Drag any tool icon from a side strip and drop it on the top half of either strip. The tool opens as a full-height column beside the viewer.",
    anchor: "right-top",
    label: moveLabel("right-top"),
  },
  {
    demo: "horizontal",
    title: "Drag horizontally",
    body: "Drop it on the bottom half of a strip instead to open it under the viewer. Two bottom tools sit side by side.",
    anchor: "right-bottom",
    label: moveLabel("right-bottom"),
  },
];

export interface OnboardingRect {
  left: number;
  top: number;
  width: number;
  height: number;
}

export interface OnboardingViewport {
  width: number;
  height: number;
}

export interface OnboardingPlacement {
  left: number;
  top: number;
  width: number;
  maxHeight: number;
  arrow: "right" | "none";
}

export interface TipPlacement {
  left: number;
  top: number;
  width: number;
  maxHeight: number;
  arrow: "left" | "right";
  arrowY: number;
}

export function shouldShowDockOnboarding(storage: Pick<Storage, "getItem"> | null, key = DOCK_ONBOARDING_KEY): boolean {
  if (storage === null) return true;
  try {
    return storage.getItem(key) === null;
  } catch {
    return true;
  }
}

export function markDockOnboardingDone(storage: Pick<Storage, "setItem"> | null, key = DOCK_ONBOARDING_KEY): void {
  if (storage === null) return;
  try {
    storage.setItem(key, DOCK_ONBOARDING_DONE);
  } catch { }
}

export function movedToAnotherAnchor(before: DockLayout, after: DockLayout, tool: DockToolId): boolean {
  return anchorOf(before, tool) !== anchorOf(after, tool);
}

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(value, max));
}

interface MeasuredBox {
  getBoundingClientRect(): { top: number; bottom: number };
}

export interface OnboardingToolbarHost extends MeasuredBox {
  querySelector(selector: string): MeasuredBox | null;
  querySelectorAll(selector: string): Iterable<MeasuredBox>;
}

export function viewerToolbarBottom(viewer: OnboardingToolbarHost): number {
  const header = viewer.querySelector("[data-dock-viewport]");
  let bottom = (header ?? viewer).getBoundingClientRect().top;
  for (const row of viewer.querySelectorAll(".ab-viewer-toolbar")) bottom = Math.max(bottom, row.getBoundingClientRect().bottom);
  return bottom;
}

export function onboardingPlacement(viewer: OnboardingRect, viewport: OnboardingViewport, toolbarBottom: number): OnboardingPlacement {
  const width = Math.round(Math.min(ONBOARDING_MAX_WIDTH_PX, Math.max(0, viewer.width - 2 * ONBOARDING_INSET_PX)));
  const narrow = viewer.width < ONBOARDING_NARROW_VIEWER_PX;
  const preferredLeft = narrow ? viewer.left + ONBOARDING_INSET_PX : viewer.left + viewer.width - ONBOARDING_INSET_PX - width;
  const left = Math.round(clamp(preferredLeft, 0, Math.max(0, viewport.width - width)));
  const below = Math.max(viewer.top, toolbarBottom) + ONBOARDING_INSET_PX;
  const top = Math.round(clamp(below, 0, Math.max(0, viewport.height - ONBOARDING_MIN_HEIGHT_PX)));
  const room = Math.min(viewer.top + viewer.height - ONBOARDING_INSET_PX - top, viewport.height - top);
  return { left, top, width, maxHeight: Math.round(Math.max(ONBOARDING_MIN_HEIGHT_PX, room)), arrow: narrow ? "none" : "right" };
}

export function tipPlacement(anchor: OnboardingRect, side: DockSide, viewport: OnboardingViewport, cardHeight: number): TipPlacement {
  const anchorRight = anchor.left + anchor.width;
  const room = side === "left" ? viewport.width - ONBOARDING_INSET_PX - anchorRight - TIP_GAP_PX : anchor.left - TIP_GAP_PX - ONBOARDING_INSET_PX;
  const width = Math.max(0, Math.min(TIP_MAX_WIDTH_PX, room));
  const left = side === "left" ? anchorRight + TIP_GAP_PX : anchor.left - TIP_GAP_PX - width;
  const maxHeight = Math.max(0, viewport.height - 2 * ONBOARDING_INSET_PX);
  const height = Math.min(Math.max(0, cardHeight), maxHeight);
  const centre = anchor.top + anchor.height / 2;
  const top = clamp(centre - height / 2, ONBOARDING_INSET_PX, viewport.height - ONBOARDING_INSET_PX - height);
  const arrowY = clamp(centre - top, TIP_ARROW_EDGE_PX, height - TIP_ARROW_EDGE_PX);
  return {
    left: Math.round(left),
    top: Math.round(top),
    width: Math.round(width),
    maxHeight: Math.round(maxHeight),
    arrow: side,
    arrowY: Math.round(arrowY),
  };
}
