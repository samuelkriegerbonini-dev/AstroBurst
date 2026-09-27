import { describe, expect, it } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { CompositeProvider } from "../../../context/CompositeContext";
import { PreviewProvider } from "../../../context/PreviewContext";
import MaskedStretchPanel from "../MaskedStretchPanel";
import { stretchPassesSummary } from "../stretchPasses";

const HINT = "More steps reach the same background with stronger star protection";

function channel(iterations_run: number, converged: boolean) {
  return { iterations_run, converged, final_background: 0.25 };
}

function renderPanel(compositeMode: boolean): string {
  const element = createElement(MaskedStretchPanel, {
    selectedFile: null,
    fileKey: null,
    compositeMode,
    compositeInput: null,
    onCompositeDone: () => {},
    fileName: "",
  });
  const preview = createElement(PreviewProvider, { file: null, doneFiles: [], children: element });
  return renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
}

describe("stretchPassesSummary", () => {
  it("reports the passes of a converged file run", () => {
    expect(stretchPassesSummary({ iterations_run: 1, converged: true })).toEqual({ text: "Stretch passes: 1 (converged)", converged: true });
  });

  it("says the run stopped before the target when the last pass missed the tolerance", () => {
    expect(stretchPassesSummary({ iterations_run: 50, converged: false })).toEqual({ text: "Stretch passes: 50 (stopped before the target)", converged: false });
    expect(stretchPassesSummary({ iterations_run: 3 })).toEqual({ text: "Stretch passes: 3 (stopped before the target)", converged: false });
  });

  it("reports per-channel passes for a composite run", () => {
    const all = stretchPassesSummary({ channels: { r: channel(1, true), g: channel(2, true), b: channel(1, true) } });
    expect(all).toEqual({ text: "Stretch passes: R 1 · G 2 · B 1 (converged)", converged: true });
    const none = stretchPassesSummary({ channels: { r: channel(10, false), g: channel(10, false), b: channel(10, false) } });
    expect(none).toEqual({ text: "Stretch passes: R 10 · G 10 · B 10 (stopped before the target)", converged: false });
    const some = stretchPassesSummary({ channels: { r: channel(1, true), g: channel(10, false), b: channel(1, true) } });
    expect(some).toEqual({ text: "Stretch passes: R 1 · G 10 · B 1 (stopped before the target: G)", converged: false });
  });

  it("never mentions a cap", () => {
    const texts = [
      stretchPassesSummary({ iterations_run: 50, converged: false }),
      stretchPassesSummary({ channels: { r: channel(10, false), g: channel(10, false), b: channel(10, false) } }),
      stretchPassesSummary({ channels: { r: channel(1, true), g: channel(10, false), b: channel(1, true) } }),
    ].map((s) => s?.text ?? "");
    for (const text of texts) expect(text).not.toMatch(/cap/);
  });

  it("prefers the channel stats over the top-level fields", () => {
    const text = stretchPassesSummary({ iterations_run: 9, converged: false, channels: { r: channel(1, true), g: channel(1, true), b: channel(1, true) } });
    expect(text?.text).toBe("Stretch passes: R 1 · G 1 · B 1 (converged)");
  });

  it("stays silent without a pass count", () => {
    expect(stretchPassesSummary({})).toBeNull();
  });
});

describe("MaskedStretchPanel iterations slider", () => {
  it.each([false, true])("is labelled Iterations in composite mode = %s", (compositeMode) => {
    const html = renderPanel(compositeMode);
    expect(html).toMatch(/>Iterations</);
    expect(html).not.toMatch(/Max iterations/);
  });

  it.each([false, true])("shows the step hint under the slider in composite mode = %s", (compositeMode) => {
    const html = renderPanel(compositeMode);
    const label = html.indexOf(">Iterations<");
    const range = html.indexOf('type="range"', label);
    const hint = html.indexOf(`>${HINT}<`);
    const next = html.indexOf(">Target Background<");
    expect(label).toBeGreaterThanOrEqual(0);
    expect(range).toBeGreaterThan(label);
    expect(hint).toBeGreaterThan(range);
    expect(next).toBeGreaterThan(hint);
  });

  it.each([false, true])("keeps the 1-50 range with the default of 10 in composite mode = %s", (compositeMode) => {
    const html = renderPanel(compositeMode);
    const start = html.indexOf(">Iterations<");
    const input = html.slice(html.indexOf("<input", start), html.indexOf("/>", html.indexOf("<input", start)));
    expect(input).toMatch(/type="range"/);
    expect(input).toMatch(/min="1"/);
    expect(input).toMatch(/max="50"/);
    expect(input).toMatch(/value="10"/);
  });
});
