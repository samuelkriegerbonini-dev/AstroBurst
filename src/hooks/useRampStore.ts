import { useSyncExternalStore } from "react";

type Listener = () => void;

export class RampStore {
  private integration = 0;
  private listeners = new Set<Listener>();

  subscribe = (listener: Listener): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  getSnapshot = (): number => this.integration;

  setIntegration(value: number) {
    const next = Number.isInteger(value) && value >= 0 ? value : 0;
    if (next === this.integration) return;
    this.integration = next;
    this.listeners.forEach((l) => l());
  }

  reset() {
    this.setIntegration(0);
  }
}

export const rampStore = new RampStore();

export function setIntegration(value: number) {
  rampStore.setIntegration(value);
}

export function resetRampIntegration() {
  rampStore.reset();
}

export function getRampIntegration(): number {
  return rampStore.getSnapshot();
}

export function useRampIntegration(): number {
  return useSyncExternalStore(rampStore.subscribe, rampStore.getSnapshot);
}
