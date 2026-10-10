import { anchorOf, moveLabel, type DockAnchor, type DockLayout, type DockToolId } from "./dockLayout";

export const DOCK_ONBOARDING_KEY = "ab.onboarding.dock.v1";
export const DOCK_ONBOARDING_DONE = "done";
export const DOCK_ONBOARDING_DELAY_MS = 600;
export const DOCK_ONBOARDING_TITLE = "Arrange your tool windows";
export const DOCK_ONBOARDING_TIP = "Right-click a tool (or Shift+F10) for Move to…, and Reset Layout puts everything back.";

export const ONBOARDING_INSET_PX = 16;
export const ONBOARDING_MAX_WIDTH_PX = 360;
export const ONBOARDING_NARROW_VIEWER_PX = 420;
export const ONBOARDING_MIN_HEIGHT_PX = 120;

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

export interface OnboardingViewerRect {
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

export function shouldShowDockOnboarding(storage: Pick<Storage, "getItem"> | null): boolean {
  if (storage === null) return true;
  try {
    return storage.getItem(DOCK_ONBOARDING_KEY) === null;
  } catch {
    return true;
  }
}

export function markDockOnboardingDone(storage: Pick<Storage, "setItem"> | null): void {
  if (storage === null) return;
  try {
    storage.setItem(DOCK_ONBOARDING_KEY, DOCK_ONBOARDING_DONE);
  } catch { }
}

export function movedToAnotherAnchor(before: DockLayout, after: DockLayout, tool: DockToolId): boolean {
  return anchorOf(before, tool) !== anchorOf(after, tool);
}

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(value, max));
}

export function onboardingPlacement(viewer: OnboardingViewerRect, viewport: OnboardingViewport): OnboardingPlacement {
  const width = Math.round(Math.min(ONBOARDING_MAX_WIDTH_PX, Math.max(0, viewer.width - 2 * ONBOARDING_INSET_PX)));
  const narrow = viewer.width < ONBOARDING_NARROW_VIEWER_PX;
  const preferredLeft = narrow ? viewer.left + ONBOARDING_INSET_PX : viewer.left + viewer.width - ONBOARDING_INSET_PX - width;
  const left = Math.round(clamp(preferredLeft, 0, Math.max(0, viewport.width - width)));
  const top = Math.round(clamp(viewer.top + ONBOARDING_INSET_PX, 0, Math.max(0, viewport.height - ONBOARDING_MIN_HEIGHT_PX)));
  const room = Math.min(viewer.height - 2 * ONBOARDING_INSET_PX, viewport.height - top);
  return { left, top, width, maxHeight: Math.round(Math.max(ONBOARDING_MIN_HEIGHT_PX, room)), arrow: narrow ? "none" : "right" };
}
