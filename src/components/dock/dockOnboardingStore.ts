import { useSyncExternalStore } from "react";
import type { DockLayout, DockToolId } from "../../utils/dockLayout";
import {
  DOCK_ONBOARDING_DELAY_MS,
  DOCK_ONBOARDING_STEPS,
  ONBOARDING_KEYS,
  ONBOARDING_ORDER,
  markDockOnboardingDone,
  movedToAnotherAnchor,
  nextOnboardingCard,
  shouldShowDockOnboarding,
  type OnboardingCardId,
} from "../../utils/dockOnboarding";

export type OnboardingStorage = Pick<Storage, "getItem" | "setItem">;
export type OnboardingSchedule = (run: () => void, ms: number) => () => void;

export interface DockOnboardingState {
  card: OnboardingCardId | null;
  step: number;
}

export interface DockOnboardingStore {
  subscribe(cb: () => void): () => void;
  get(): DockOnboardingState;
  autoShow(): void;
  show(): void;
  setStep(step: number): void;
  dismiss(): void;
  skipAll(): void;
  setReady(card: OnboardingCardId, ready: boolean): void;
  setDragging(dragging: boolean): void;
  setSettingsOpen(open: boolean): void;
  stripToggled(tool: DockToolId): void;
  afterMove(before: DockLayout, after: DockLayout, tool: DockToolId): void;
}

const CLOSED: DockOnboardingState = { card: null, step: 0 };

function browserOnboardingStorage(): OnboardingStorage | null {
  try {
    return globalThis.localStorage ?? null;
  } catch {
    return null;
  }
}

function timeoutSchedule(run: () => void, ms: number): () => void {
  const timer = setTimeout(run, ms);
  return () => clearTimeout(timer);
}

export function createDockOnboardingStore(storage: () => OnboardingStorage | null, schedule: OnboardingSchedule = timeoutSchedule): DockOnboardingStore {
  let state = CLOSED;
  let started = false;
  let dragging = false;
  let settingsOpen = false;
  let cancelPause: (() => void) | null = null;
  const done = new Set<OnboardingCardId>();
  const ready: Record<OnboardingCardId, boolean> = { toggle: false, settings: false, tour: true };
  const listeners = new Set<() => void>();

  const set = (next: DockOnboardingState) => {
    if (next.card === state.card && next.step === state.step) return;
    state = next;
    listeners.forEach((l) => l());
  };

  const nextCard = () => (dragging ? null : nextOnboardingCard(done, ready));

  const showNext = () => {
    if (!started || state.card !== null || cancelPause !== null) return;
    const card = nextCard();
    if (card !== null) set({ card, step: 0 });
  };

  const stopPause = () => {
    cancelPause?.();
    cancelPause = null;
  };

  const closeAndPause = () => {
    set(CLOSED);
    stopPause();
    cancelPause = schedule(() => {
      cancelPause = null;
      showNext();
    }, DOCK_ONBOARDING_DELAY_MS);
  };

  const complete = (card: OnboardingCardId) => {
    done.add(card);
    markDockOnboardingDone(storage(), ONBOARDING_KEYS[card]);
    if (state.card === card) closeAndPause();
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
      if (started) return;
      started = true;
      const saved = storage();
      for (const card of ONBOARDING_ORDER) {
        if (!shouldShowDockOnboarding(saved, ONBOARDING_KEYS[card])) done.add(card);
      }
      showNext();
    },
    show() {
      started = true;
      done.clear();
      stopPause();
      const card = nextCard();
      set(card === null ? CLOSED : { card, step: 0 });
    },
    setStep(step) {
      if (state.card !== "tour") return;
      set({ card: "tour", step: Math.min(Math.max(0, Math.trunc(step)), DOCK_ONBOARDING_STEPS.length - 1) });
    },
    dismiss() {
      const card = state.card;
      if (card !== null) complete(card);
    },
    skipAll() {
      const saved = storage();
      for (const card of ONBOARDING_ORDER) {
        done.add(card);
        markDockOnboardingDone(saved, ONBOARDING_KEYS[card]);
      }
      stopPause();
      set(CLOSED);
    },
    setReady(card, value) {
      if (ready[card] === value) return;
      ready[card] = value;
      if (!value && state.card === card) closeAndPause();
      else showNext();
    },
    setDragging(value) {
      dragging = value;
      showNext();
    },
    setSettingsOpen(open) {
      const opened = open && !settingsOpen;
      settingsOpen = open;
      if (opened && state.card === "settings") complete("settings");
    },
    stripToggled(tool) {
      const card = state.card;
      if (card === "toggle" || (card === "settings" && tool === "config")) complete(card);
    },
    afterMove(before, after, tool) {
      if (movedToAnotherAnchor(before, after, tool)) complete("tour");
    },
  };
}

export const dockOnboardingStore = createDockOnboardingStore(browserOnboardingStorage);

export function useDockOnboarding(): DockOnboardingState {
  return useSyncExternalStore(dockOnboardingStore.subscribe, dockOnboardingStore.get, dockOnboardingStore.get);
}
