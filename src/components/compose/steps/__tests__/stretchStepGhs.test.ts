import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

interface SliderProps {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  format?: (v: number) => string;
}

const { sliders, shared } = vi.hoisted(() => ({
  sliders: [] as SliderProps[],
  shared: {
    defaults: { stretchFactor: 3.5, localIntensity: 1.5, symmetryPoint: 0.004, shadowProtect: 0.001, highlightProtect: 0.95 },
    limits: {
      stretchFactor: { min: 0, max: 40, step: 0.1 },
      localIntensity: { min: -4, max: 12, step: 0.5 },
      symmetryPoint: { min: 0, max: 0.5, step: 0.0005 },
      shadowProtect: { min: 0, max: 0.5, step: 0.0005 },
      highlightProtect: { min: 0.5, max: 1, step: 0.0005 },
    },
  },
}));

vi.mock("../../../ui", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../ui")>()),
  Slider: (props: SliderProps) => {
    sliders.push(props);
    return null;
  },
  RunButton: () => null,
}));

vi.mock("../../../../services/processing", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../../services/processing")>()),
  GHS_DEFAULTS: shared.defaults,
  GHS_LIMITS: shared.limits,
}));

vi.mock("../../../../context/CompositeContext", () => {
  const stf = { shadow: 0, midtone: 0.5, highlight: 1 };
  return {
    useCompositeStf: () => ({
      compositeStfR: stf,
      compositeStfG: stf,
      compositeStfB: stf,
      compositeStfLinked: true,
      compositeAutoStfR: stf,
      compositeAutoStfG: stf,
      compositeAutoStfB: stf,
      wizardStfR: stf,
      wizardStfG: stf,
      wizardStfB: stf,
      wizardStfLinked: true,
    }),
    useCompositeActions: () => ({ setCompositeStf: vi.fn(), setCompositeStfLinked: vi.fn() }),
  };
});

vi.mock("../../../../context/PreviewContext", () => ({
  useRenderActions: () => ({ currentFileKey: () => null, publishProcessed: vi.fn() }),
}));

vi.mock("../../StfHistogram", () => ({ default: () => null }));

import StretchStep from "../StretchStep";
import GhsStretchControls from "../../../processing/GhsStretchControls";
import { INITIAL_STATE, type WizardState } from "../../../../utils/wizard";

const HA_FILE = { ha: ["/d/656nmos.fits"] };

function ghsState(): WizardState {
  return {
    ...INITIAL_STATE,
    bins: INITIAL_STATE.bins.map((b) => ({ ...b, files: HA_FILE[b.id as keyof typeof HA_FILE] ?? [] })),
    stretchMode: "ghs",
  };
}

interface Rendered {
  sliders: SliderProps[];
  html: string;
}

function rendered(element: Parameters<typeof renderToStaticMarkup>[0]): Rendered {
  sliders.length = 0;
  const html = renderToStaticMarkup(element);
  return { sliders: [...sliders], html };
}

function composeGhs(): Rendered {
  return rendered(createElement(StretchStep, {
    state: ghsState(),
    onStretchChange: vi.fn(),
    onMaskParams: vi.fn(),
    onMask: vi.fn(),
    onResult: vi.fn(),
    onChannelOutput: vi.fn(),
    onCompositeOp: vi.fn(),
  }));
}

function processingGhs(): Rendered {
  return rendered(createElement(GhsStretchControls, { settings: shared.defaults, disabled: false, onChange: vi.fn() }));
}

function shape(list: SliderProps[]) {
  return list.map(({ label, value, min, max, step, format }) => ({ label, value, min, max, step, shown: format?.(value) }));
}

function ghsHint(html: string): string | undefined {
  return />(On linear data set SP[^<]*)</.exec(html)?.[1];
}

describe("Compose › Stretch GHS controls", () => {
  it("take their ranges and starting values from the shared GHS limits and defaults, like Processing › Stretch", () => {
    const compose = shape(composeGhs().sliders);
    expect(compose.map((s) => s.label)).toEqual([
      "Stretch (D)",
      "Local Intensity (b)",
      "Symmetry Point (SP)",
      "Shadow Protect (LP)",
      "Highlight Protect (HP)",
    ]);
    expect(compose).toEqual(shape(processingGhs().sliders));
    expect(compose[2]).toEqual({ label: "Symmetry Point (SP)", value: 0.004, min: 0, max: 0.5, step: 0.0005, shown: "0.004" });
  });

  it("show the same hint as Processing › Stretch", () => {
    const hint = ghsHint(composeGhs().html);
    expect(hint).toBeDefined();
    expect(hint).toBe(ghsHint(processingGhs().html));
  });
});
