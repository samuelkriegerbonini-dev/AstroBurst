import { useSyncExternalStore } from "react";
import type { DockLayout, DockToolId } from "../../utils/dockLayout";
import {
  DOCK_ONBOARDING_STEPS,
  markDockOnboardingDone,
  movedToAnotherAnchor,
  shouldShowDockOnboarding,
} from "../../utils/dockOnboarding";

export type OnboardingStorage = Pick<Storage, "getItem" | "setItem">;

export interface DockOnboardingState {
  open: boolean;
  step: number;
}

export interface DockOnboardingStore {
  subscribe(cb: () => void): () => void;
  get(): DockOnboardingState;
  autoShow(): void;
  show(): void;
  setStep(step: number): void;
  dismiss(): void;
  afterMove(before: DockLayout, after: DockLayout, tool: DockToolId): void;
}

const CLOSED: DockOnboardingState = { open: false, step: 0 };
const FIRST_STEP: DockOnboardingState = { open: true, step: 0 };

function browserOnboardingStorage(): OnboardingStorage | null {
  try {
    return globalThis.localStorage ?? null;
  } catch {
    return null;
  }
}

export function createDockOnboardingStore(storage: () => OnboardingStorage | null): DockOnboardingStore {
  let state = CLOSED;
  let autoSettled = false;
  const listeners = new Set<() => void>();

  const set = (next: DockOnboardingState) => {
    if (next.open === state.open && next.step === state.step) return;
    state = next;
    listeners.forEach((l) => l());
  };

  const dismiss = () => {
    autoSettled = true;
    markDockOnboardingDone(storage());
    set(CLOSED);
  };

  return {
    subscribe(cb) {
      listeners.add(cb);
      return () => {
        listeners.delete(cb);
      };
    },
    get: () => state,
    autoShow() {
      if (autoSettled) return;
      autoSettled = true;
      if (shouldShowDockOnboarding(storage())) set(FIRST_STEP);
    },
    show() {
      set(FIRST_STEP);
    },
    setStep(step) {
      if (!state.open) return;
      set({ open: true, step: Math.min(Math.max(0, Math.trunc(step)), DOCK_ONBOARDING_STEPS.length - 1) });
    },
    dismiss,
    afterMove(before, after, tool) {
      if (movedToAnotherAnchor(before, after, tool)) dismiss();
    },
  };
}

export const dockOnboardingStore = createDockOnboardingStore(browserOnboardingStorage);

export function useDockOnboarding(): DockOnboardingState {
  return useSyncExternalStore(dockOnboardingStore.subscribe, dockOnboardingStore.get, dockOnboardingStore.get);
}
