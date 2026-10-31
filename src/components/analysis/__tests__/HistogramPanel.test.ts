import { describe, expect, it } from "vitest";
import { createElement, type ComponentProps } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import HistogramPanel, { ManualInput } from "../HistogramPanel";
import histogramPanelSource from "../HistogramPanel.tsx?raw";
import { STF_MIDTONE_FIELD, stfDraft, stfDraftFields } from "../../../utils/analysisTarget";

type PanelProps = ComponentProps<typeof HistogramPanel>;

function render(props: Partial<PanelProps> = {}): string {
  return renderToStaticMarkup(
    createElement(HistogramPanel, {
      bins: [1, 4, 9, 4, 1],
      dataMin: 0,
      dataMax: 100,
      stats: { mean: 40, median: 38, sigma: 3 },
      onChange: () => {},
      onSkyZoomChange: () => {},
      ...props,
    }),
  );
}

describe("HistogramPanel channel badge (preview0923-98)", () => {
  it("labels the statistics of an RGB file as the R plane", () => {
    const markup = render({ channel: "R" });
    expect(markup).toContain(">R channel</span>");
    expect(markup).toContain('title="Statistics of the R plane only"');
    expect(markup).toContain('data-histogram-channel="R"');
  });

  it("shows no channel badge for a mono histogram", () => {
    const markup = render();
    expect(markup).not.toContain("channel</span>");
    expect(markup).not.toContain("data-histogram-channel");
  });
});

describe("HistogramPanel grey tokens (audit0921-74/80)", () => {
  it("draws the off-state Full/Sky and toolbar buttons with the --ab-text-3 token", () => {
    for (const skyZoom of [false, true]) {
      const markup = render({ skyZoom, skyAvailable: true });
      expect(markup).toContain("color:var(--ab-text-3)");
      expect(markup).not.toMatch(/#71717a|#52525b/i);
    }
  });
});

describe("HistogramPanel manual S/M/H fields (followups-9)", () => {
  const SHADOW = { optional: true, min: 0, max: 1 };

  function field(value: string): string {
    return renderToStaticMarkup(
      createElement(ManualInput, { label: "S", name: "S shadow clipping point (0-1)", color: "#ef4444", value, field: SHADOW, onChange: () => {} }),
    );
  }

  it("render a decimal text box named by its aria-label", () => {
    const markup = field("0.0125");
    expect(markup).toContain('type="text"');
    expect(markup).toContain('inputMode="decimal"');
    expect(markup).toContain('aria-label="S shadow clipping point (0-1)"');
    expect(markup).not.toContain("aria-invalid");
  });

  it("mark a decimal comma invalid instead of reading 0,5 as 0", () => {
    const markup = field("0,5");
    expect(markup).toContain('aria-invalid="true"');
    expect(markup).toContain('title="Use a dot (.) as the decimal separator"');
  });

  it("mark a value outside 0-1 invalid and leave a blank field to its default", () => {
    expect(field("1.5")).toContain('title="Use a value from 0 to 1"');
    expect(field("")).not.toContain("aria-invalid");
  });

  it("leave no native number input or parseFloat in the panel", () => {
    expect(histogramPanelSource).not.toContain('type="number"');
    expect(histogramPanelSource).not.toMatch(/parseFloat\(/);
  });
});

describe("HistogramPanel manual prefill of the current STF (followups-9 repair)", () => {
  function midtoneField(value: string): string {
    return renderToStaticMarkup(
      createElement(ManualInput, { label: "M", name: "M midtone balance (0-1)", color: "#eab308", value, field: STF_MIDTONE_FIELD, onChange: () => {} }),
    );
  }

  it("opens Manual on the JWST NIRCam auto-STF midtone 0.000111 with a valid M field and Apply enabled", () => {
    const draft = stfDraft({ shadow: 0.0042, midtone: 0.000111, highlight: 1 });
    expect(draft.midtone).toBe("0.000111");
    expect(midtoneField(draft.midtone)).not.toContain("aria-invalid");
    expect(stfDraftFields(draft).error).toBeNull();
    expect(stfDraftFields(draft).fields.midtone.value).toBe(0.000111);
  });

  it("prefills every midtone the backend applies, from its 0.0001 clamp to 0.9999, as a valid value", () => {
    for (const midtone of [0.0001, 0.00042, 0.0168, 0.5, 0.99995, 0.9999]) {
      const draft = stfDraft({ shadow: 0, midtone, highlight: 1 });
      expect(stfDraftFields(draft).error).toBeNull();
      expect(Math.abs(Number(draft.midtone) - midtone) / midtone).toBeLessThan(1e-3);
    }
  });

  it("prefills a sky-zoom drag below the backend clamp with the 0.0001 the backend applies", () => {
    const draft = stfDraft({ shadow: 0.0046, midtone: 3.6e-6, highlight: 1 });
    expect(draft.midtone).toBe("0.0001");
    expect(stfDraftFields(draft).error).toBeNull();
  });

  it("still refuses a typed midtone outside the backend range", () => {
    const draft = { shadow: "0", midtone: "0", highlight: "1" };
    expect(stfDraftFields(draft).error).toBe("M: Use a value from 0.0001 to 0.9999");
  });
});

describe("HistogramPanel STF state readout (SF-01)", () => {
  const AUTO = { shadow: 0.004321, midtone: 0.008467, highlight: 1 };
  const LOCK = "STF applies only to the mtf stretch";

  function buttonWith(markup: string, inner: string): string {
    return markup.split("<button").find((chunk) => chunk.includes(inner)) ?? "";
  }

  it("starts the readout with 'Auto STF' and presses Auto in its active colour when the STF is the auto STF", () => {
    const markup = render({ autoStf: AUTO, ...AUTO });
    expect(markup).toContain('data-stf-state="auto"');
    expect(markup).toContain(">Auto STF</div>");
    expect(markup.indexOf("Auto STF")).toBeLessThan(markup.indexOf("S: "));
    const auto = buttonWith(markup, "<span>Auto</span>");
    expect(auto).toContain('aria-pressed="true"');
    expect(auto).toContain("color:var(--ab-blue)");
  });

  it("says 'Linear' for 0 / 0.5 / 1 and leaves Auto unpressed", () => {
    const markup = render({ autoStf: AUTO, shadow: 0, midtone: 0.5, highlight: 1 });
    expect(markup).toContain('data-stf-state="linear"');
    expect(markup).toContain(">Linear</div>");
    expect(markup).not.toContain("Auto STF");
    const auto = buttonWith(markup, "<span>Auto</span>");
    expect(auto).toContain('aria-pressed="false"');
    expect(auto).toContain("color:var(--ab-text-3)");
  });

  it("says 'Manual STF' once a marker moved off the auto STF", () => {
    const markup = render({ autoStf: AUTO, ...AUTO, midtone: 0.0121 });
    expect(markup).toContain('data-stf-state="manual"');
    expect(markup).toContain(">Manual STF</div>");
    expect(buttonWith(markup, "<span>Auto</span>")).toContain('aria-pressed="false"');
  });

  it("names the icon buttons and titles Reset 'Linear (no stretch)' instead of 'Reset to linear'", () => {
    const markup = render({ autoStf: AUTO, ...AUTO });
    expect(buttonWith(markup, 'aria-label="Linear (no stretch)"')).toContain('title="Linear (no stretch)"');
    expect(buttonWith(markup, 'aria-label="Manual input"')).toContain('title="Manual input"');
    expect(markup).not.toContain("Reset to linear");
  });

  it("keeps the icon buttons' names and the state word while the lock hint takes the titles", () => {
    const markup = render({ autoStf: AUTO, ...AUTO, disabled: true, disabledHint: LOCK });
    expect(buttonWith(markup, 'aria-label="Linear (no stretch)"')).toContain(`title="${LOCK}"`);
    expect(buttonWith(markup, 'aria-label="Manual input"')).toContain(`title="${LOCK}"`);
    expect(markup).toContain('data-stf-state="auto"');
  });
});
