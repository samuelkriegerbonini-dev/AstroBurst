import { useCallback, useSyncExternalStore } from "react";
import type { ContinuumWindows } from "../shared/types/cube";
import type { LineMeasurement, LineModel, SpectralAxisMode, VelocityConvention } from "../shared/types/spectral";
import type { FluxDisplayChoice, Spectrum1d, WavelengthDisplayUnit, WavelengthFrameChoice } from "../shared/types/spectrum1d";
import { LINE_FAMILIES, type LineFamily } from "../utils/lineList";
import { defaultFrameChoice } from "../utils/spectrumDocument";
import type { ChannelRange } from "../utils/spectrumRange";
import type { SpectrumSetup } from "../utils/spectrumSetup";

export type YLimitsMode = "full" | "robust";

export interface SpectrumViewState {
  mode: SpectralAxisMode | null;
  frameChoice: WavelengthFrameChoice | null;
  restUm: number | null;
  convention: VelocityConvention;
  redshift: number;
  lineListVisible: boolean;
  lineFamilies: LineFamily[];
  bandSpecies: string[];
  pickedLineId: string | null;
  lineModel: LineModel;
  useDq: boolean;
  range: ChannelRange | null;
  windows: ContinuumWindows | null;
  wavelengthUnit: WavelengthDisplayUnit;
  fluxUnit: FluxDisplayChoice;
  binFactor: number;
  logY: boolean;
  yLimits: YLimitsMode;
  xDomain: [number, number] | null;
}

export const DEFAULT_SPECTRUM_VIEW: Readonly<SpectrumViewState> = Object.freeze({
  mode: null,
  frameChoice: null,
  restUm: null,
  convention: "optical",
  redshift: 0,
  lineListVisible: true,
  lineFamilies: [...LINE_FAMILIES],
  bandSpecies: [],
  pickedLineId: null,
  lineModel: "gaussian",
  useDq: true,
  range: null,
  windows: null,
  wavelengthUnit: "um",
  fluxUnit: "native",
  binFactor: 1,
  logY: false,
  yLimits: "full",
  xDomain: null,
});

export const MAX_SPECTRUM_VIEWS = 16;

export interface SpectrumLineOverlay {
  result: LineMeasurement;
  measurementAxis: number[];
}

const DISPLAYED_AXIS_FIELDS: readonly (keyof SpectrumViewState)[] = ["mode", "frameChoice", "wavelengthUnit", "convention"];

type Listener = () => void;

export function documentViewKey(path: string, selection: string): string {
  return `${selection}|${path}`;
}

export function cubeViewKey(path: string): string {
  return `cube|${path}`;
}

function sameValue(a: unknown, b: unknown): boolean {
  if (Object.is(a, b)) return true;
  if (!Array.isArray(a) || !Array.isArray(b) || a.length !== b.length) return false;
  return a.every((value, i) => sameValue(value, b[i]));
}

function changedFields(current: SpectrumViewState, next: SpectrumViewState): (keyof SpectrumViewState)[] {
  return (Object.keys(next) as (keyof SpectrumViewState)[]).filter((field) => !sameValue(current[field], next[field]));
}

export class SpectrumViewStore {
  private views = new Map<string, SpectrumViewState>();
  private overlays = new Map<string, SpectrumLineOverlay>();
  private listeners = new Set<Listener>();
  private setup: SpectrumSetup | null = null;
  private seedLabels = new Map<string, string>();
  private dismissedKeys = new Set<string>();

  subscribe = (listener: Listener): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  private notify(): void {
    this.listeners.forEach((l) => l());
  }

  get(key: string): SpectrumViewState {
    const view = this.views.get(key);
    if (!view) return DEFAULT_SPECTRUM_VIEW;
    this.views.delete(key);
    this.views.set(key, view);
    return view;
  }

  patch(key: string, partial: Partial<SpectrumViewState>): void {
    const current = this.get(key);
    const next = { ...current, ...partial };
    const changed = changedFields(current, next);
    if (changed.length === 0) return;
    const axisChanged =
      changed.some((field) => DISPLAYED_AXIS_FIELDS.includes(field)) || (changed.includes("restUm") && next.mode === "velocity");
    if (axisChanged) next.xDomain = null;
    this.views.delete(key);
    this.views.set(key, next);
    while (this.views.size > MAX_SPECTRUM_VIEWS) {
      const oldest = this.views.keys().next().value;
      if (oldest === undefined) break;
      this.views.delete(oldest);
      this.overlays.delete(oldest);
      this.seedLabels.delete(oldest);
    }
    this.notify();
  }

  overlay(key: string): SpectrumLineOverlay | null {
    return this.overlays.get(key) ?? null;
  }

  setOverlay(key: string, overlay: SpectrumLineOverlay | null): void {
    if (this.overlay(key) === overlay) return;
    if (overlay) this.overlays.set(key, overlay);
    else this.overlays.delete(key);
    this.notify();
  }

  keys(): string[] {
    return [...this.views.keys()];
  }

  lastSetup(): SpectrumSetup | null {
    return this.setup;
  }

  rememberSetup(setup: SpectrumSetup): void {
    this.setup = setup;
  }

  seededFrom(key: string): string | null {
    return this.seedLabels.get(key) ?? null;
  }

  markSeeded(key: string, label: string): void {
    this.seedLabels.set(key, label);
  }

  dismissSetup(key: string): void {
    this.setup = null;
    this.seedLabels.delete(key);
    this.dismissedKeys.add(key);
  }

  setupDismissed(key: string): boolean {
    return this.dismissedKeys.has(key);
  }

  reset(): void {
    this.views.clear();
    this.overlays.clear();
    this.setup = null;
    this.seedLabels.clear();
    this.dismissedKeys.clear();
    this.notify();
  }
}

export const spectrumViewStore = new SpectrumViewStore();

export function useSpectrumView(key: string | null): [SpectrumViewState, (partial: Partial<SpectrumViewState>) => void] {
  const getSnapshot = useCallback(() => (key === null ? DEFAULT_SPECTRUM_VIEW : spectrumViewStore.get(key)), [key]);
  const view = useSyncExternalStore(spectrumViewStore.subscribe, getSnapshot, getSnapshot);
  const patch = useCallback(
    (partial: Partial<SpectrumViewState>) => {
      if (key !== null) spectrumViewStore.patch(key, partial);
    },
    [key],
  );
  return [view, patch];
}

export function useSpectrumLineOverlay(key: string | null): SpectrumLineOverlay | null {
  const getSnapshot = useCallback(() => (key === null ? null : spectrumViewStore.overlay(key)), [key]);
  return useSyncExternalStore(spectrumViewStore.subscribe, getSnapshot, getSnapshot);
}

export function effectiveFrameChoice(view: SpectrumViewState, spectrum: Spectrum1d | null): WavelengthFrameChoice | null {
  if (spectrum?.wavelength_frame === "not_wavelength") return null;
  return view.frameChoice ?? defaultFrameChoice(spectrum?.wavelength_frame);
}

export function effectiveMode(view: SpectrumViewState, spectrum: Spectrum1d | null): SpectralAxisMode {
  if (view.mode !== null) return view.mode;
  return spectrum !== null && effectiveFrameChoice(view, spectrum) === "air" ? "wavelength_air" : "wavelength_vac";
}
