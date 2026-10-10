import { describe, expect, it, vi } from "vitest";
import { createElement, isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";

vi.mock("../../../../services/compose", () => ({
  blendChannels: vi.fn(),
  lrgbCombineComposite: vi.fn(),
  renderLinearCompositePreview: vi.fn(),
}));

vi.mock("../../../../services/channelLevels", () => ({
  measureChannelLevels: vi.fn(),
}));

vi.mock("../../../../infrastructure/tauri", () => ({
  getOutputDir: vi.fn(async () => "/out"),
}));

vi.mock("../../../../hooks/usePointingOverlap", () => ({
  usePointingOverlap: () => ({ disjointPairs: [] }),
}));

import { BlendColumnNote } from "../BlendStep";
import { wavelengthAutoWeights, wavelengthAutoWeightsBalanced, type ColorAxes } from "../../../../utils/blendWeights";
import type { FrequencyBin } from "../../../../utils/wizard";

const bin = (id: string, wavelength: number): FrequencyBin => ({
  id,
  label: id,
  shortLabel: id,
  wavelength,
  color: "#fff",
  files: ["a.fits"],
});

const TWO_FILTERS = [bin("ha", 656), bin("oiii", 501)];
const AUTO = wavelengthAutoWeights(TWO_FILTERS);
const BALANCED = wavelengthAutoWeightsBalanced(TWO_FILTERS);

function props(stfLinked: boolean | undefined, weights: ColorAxes[], balancedActive = false, onUseBalanced = vi.fn()) {
  return { stfLinked, weights, balancedActive, onUseBalanced };
}

function render(p: ReturnType<typeof props>): string {
  return renderToStaticMarkup(createElement(BlendColumnNote, p));
}

function findByTestId(node: ReactNode, testId: string): ReactElement<Record<string, unknown>> | null {
  if (Array.isArray(node)) {
    for (const child of node) {
      const hit = findByTestId(child, testId);
      if (hit) return hit;
    }
    return null;
  }
  if (!isValidElement<Record<string, unknown>>(node)) return null;
  if (node.props["data-testid"] === testId) return node;
  return findByTestId(node.props.children as ReactNode, testId);
}

describe("Blend unequal column totals note", () => {
  it("shows the note and the Balanced (λ) button when the STF was unlinked after a two-filter Auto (λ) blend", () => {
    const html = render(props(false, AUTO));

    expect(html).toContain('data-testid="blend-column-note"');
    expect(html).toContain("(R 1.00 · G 0.50 · B 0.50)");
    expect(html).toMatch(/<button[^>]*data-testid="blend-use-balanced"[^>]*>Use Balanced \(λ\)<\/button>/);
  });

  it.each([true, undefined])("renders nothing when the STF stayed linked (stf_linked %s)", (linked) => {
    expect(render(props(linked, AUTO))).toBe("");
  });

  it("renders nothing for an unlinked STF when the weights already give equal totals", () => {
    expect(render(props(false, BALANCED))).toBe("");
  });

  it("keeps the note but hides the button once Balanced (λ) is the active preset", () => {
    const html = render(props(false, AUTO, true));

    expect(html).toContain('data-testid="blend-column-note"');
    expect(html).not.toContain("blend-use-balanced");
  });

  it("wires the button to the Balanced (λ) handler", () => {
    const onUseBalanced = vi.fn();
    const button = findByTestId(BlendColumnNote(props(false, AUTO, false, onUseBalanced)), "blend-use-balanced");

    expect(button).not.toBeNull();
    (button!.props.onClick as () => void)();
    expect(onUseBalanced).toHaveBeenCalledTimes(1);
  });
});
