import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { WizardState } from "../../../../utils/wizard";

const mocks = vi.hoisted(() => ({ extractBackground: vi.fn(), extractBackgroundBatch: vi.fn() }));

vi.mock("react", async (importOriginal) => (await import("../../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));

vi.mock("../../../../services/processing", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../../services/processing")>()),
  extractBackground: mocks.extractBackground,
  extractBackgroundBatch: mocks.extractBackgroundBatch,
}));

vi.mock("../../../../infrastructure/tauri", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../../infrastructure/tauri")>()),
  getOutputDir: () => Promise.resolve("C:/out"),
}));

import { hooks } from "../../../../test/hookRuntime";
import { INITIAL_STATE } from "../../../../utils/wizard";
import BackgroundStep from "../BackgroundStep";

const BINS = ["r", "g", "b"] as const;
const M16 = { r: ["/m16/673nmos.fits"], g: ["/m16/656nmos.fits"], b: ["/m16/502nmos.fits"] };
const ALIGNED = { r: "__wizard_ch_tk1_r_aligned", g: "__wizard_ch_tk1_g_aligned", b: "__wizard_ch_tk1_b_aligned" };
const BATCH_MODES = ["linked", "neutralize", "deband_auto", "deband_rows", "deband_cols", "deband_both"];

const modelUrl = (bin: string) => `http://asset.localhost/C%3A%2Fout%2F${bin}_wizard_${bin}_bg_model.png`;
const correctedUrl = (bin: string) => `http://asset.localhost/C%3A%2Fout%2F${bin}_wizard_${bin}_bg_corrected.png`;

function perChannelResult(bin: string) {
  return {
    corrected_png: `C:/out/${bin}_wizard_${bin}_bg_corrected.png`,
    model_png: `C:/out/${bin}_wizard_${bin}_bg_model.png`,
    previewUrl: correctedUrl(bin),
    modelUrl: modelUrl(bin),
    corrected_fits: `__wizard_ch_bg_${bin}`,
    cache_key: `__wizard_ch_bg_${bin}`,
    sample_count: 29,
    rms_residual: 0.8006,
    elapsed_ms: 1400,
    dimensions: [1600, 1600],
  };
}

function alignedState(): WizardState {
  return {
    ...INITIAL_STATE,
    bins: INITIAL_STATE.bins.map((b) => ({ ...b, files: M16[b.id as keyof typeof M16] ?? [] })),
    alignedPaths: ALIGNED,
    alignRunToken: "tk1",
  };
}

interface Node {
  type: unknown;
  props: Record<string, unknown> & { children?: ReactNode };
}

function nodes(node: ReactNode): Node[] {
  if (Array.isArray(node)) return node.flatMap(nodes);
  if (!isValidElement<Node["props"]>(node)) return [];
  return [{ type: node.type, props: node.props }, ...nodes(node.props.children)];
}

function textOf(node: ReactNode): string {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  return isValidElement<Node["props"]>(node) ? textOf(node.props.children) : "";
}

const state = alignedState();
const onBackground = vi.fn();
const step = () => hooks.render(BackgroundStep, { state, onBackground }) as ReactElement;
const html = () => renderToStaticMarkup(step());

function row(markup: string, bin: string): string {
  const start = markup.indexOf(`data-bin="${bin}"`);
  expect(start, `row ${bin}`).toBeGreaterThan(-1);
  const next = markup.indexOf("data-bin=", start + 1);
  return markup.slice(start, next < 0 ? undefined : next);
}

function imageSrc(markup: string, alt: string): string | null {
  const tag = markup.match(new RegExp(`<img[^>]*alt="${alt}"[^>]*>`));
  return tag?.[0].match(/src="([^"]*)"/)?.[1] ?? null;
}

async function extractAll(): Promise<void> {
  const run = nodes(step()).find((n) => n.props.label === "Extract All");
  expect(run, "Extract All").toBeDefined();
  await (run!.props.onClick as () => Promise<void>)();
}

function chooseMode(mode: string): void {
  const select = nodes(step()).find((n) => n.type === "select");
  expect(select, "mode select").toBeDefined();
  (select!.props.onChange as (e: { target: { value: string } }) => void)({ target: { value: mode } });
}

describe("Compose BG model and corrected previews", () => {
  beforeEach(() => {
    hooks.mount();
    onBackground.mockReset();
    mocks.extractBackground.mockReset();
    mocks.extractBackgroundBatch.mockReset();
    mocks.extractBackground.mockImplementation((_path: string, _dir: string, options: { binId: string }) =>
      Promise.resolve(perChannelResult(options.binId)));
    mocks.extractBackgroundBatch.mockImplementation((_paths: string[], binIds: string[]) =>
      Promise.resolve({
        results: binIds.map((bin) => ({ bin_id: bin, cache_key: `__wizard_ch_tk1_bg_${bin}`, sample_count: 37, axis: null })),
        mode: "subtract",
        rms_residual: 1.718,
        dimensions: [1600, 1600],
        elapsed_ms: 900,
      }));
  });

  afterEach(() => hooks.mount());

  it("BG result shows model and corrected thumbnails in per-channel mode", async () => {
    expect(html()).not.toContain("<img");
    await extractAll();
    expect(mocks.extractBackground).toHaveBeenCalledTimes(3);
    expect(onBackground.mock.calls).toEqual(BINS.map((bin) => [bin, `__wizard_ch_bg_${bin}`]));
    const markup = html();
    expect(markup.match(/<img[^>]*alt="Model"/g) ?? [], "model thumbnails").toHaveLength(3);
    expect(markup.match(/<img[^>]*alt="Corrected"/g) ?? [], "corrected thumbnails").toHaveLength(3);
    for (const bin of BINS) {
      const channel = row(markup, bin);
      expect(channel).toContain("Model / Corrected");
      expect(imageSrc(channel, "Model")).toMatch(new RegExp(`^${modelUrl(bin).replace(/[.?]/g, "\\$&")}\\?v=\\d+$`));
      expect(imageSrc(channel, "Corrected")).toMatch(new RegExp(`^${correctedUrl(bin).replace(/[.?]/g, "\\$&")}\\?v=\\d+$`));
    }
    expect(markup).not.toContain("Preview in per-channel mode only");
  });

  it("one channel's Extract shows that channel's pair only", async () => {
    const extract = nodes(step()).filter((n) => n.type === "button" && textOf(n.props.children) === "Extract");
    expect(extract).toHaveLength(3);
    await (extract[1].props.onClick as () => Promise<void>)();
    const markup = html();
    expect(imageSrc(row(markup, "g"), "Model")).not.toBeNull();
    expect(imageSrc(row(markup, "r"), "Model")).toBeNull();
    expect(imageSrc(row(markup, "b"), "Model")).toBeNull();
  });

  it("a Re-extract hands CompareView new URLs, so the rewritten PNGs are not served from the cache", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    try {
      const pairOf = (bin: number) => nodes(step()).filter((n) => n.props.originalLabel === "Model")[bin]?.props;
      vi.setSystemTime(1000);
      await extractAll();
      expect(pairOf(0)).toMatchObject({ originalUrl: `${modelUrl("r")}?v=1000`, resultUrl: `${correctedUrl("r")}?v=1000` });
      vi.setSystemTime(2000);
      const reExtract = nodes(step()).filter((n) => n.type === "button" && textOf(n.props.children) === "Re-extract");
      expect(reExtract).toHaveLength(3);
      await (reExtract[0].props.onClick as () => Promise<void>)();
      expect(pairOf(0)).toMatchObject({ originalUrl: `${modelUrl("r")}?v=2000`, resultUrl: `${correctedUrl("r")}?v=2000` });
      expect(pairOf(1)).toMatchObject({ originalUrl: `${modelUrl("g")}?v=1000` });
    } finally {
      vi.useRealTimers();
    }
  });

  it("enlarges one channel's pair on click and shrinks it on a second click", async () => {
    await extractAll();
    const sizes = () => nodes(step()).filter((n) => n.props["data-testid"] === "bg-preview").map((n) => n.props["data-size"]);
    const toggle = (bin: number) => {
      const buttons = nodes(step()).filter((n) => n.type === "button" && /^(Enlarge|Shrink)$/.test(textOf(n.props.children)));
      expect(buttons).toHaveLength(3);
      (buttons[bin].props.onClick as () => void)();
    };
    expect(sizes()).toEqual(["thumbnail", "thumbnail", "thumbnail"]);
    toggle(0);
    expect(sizes()).toEqual(["large", "thumbnail", "thumbnail"]);
    expect(html()).toContain('aria-pressed="true"');
    toggle(0);
    expect(sizes()).toEqual(["thumbnail", "thumbnail", "thumbnail"]);
  });

  it("batch modes say the preview is per-channel only and replace the pairs with their own results", async () => {
    await extractAll();
    for (const mode of BATCH_MODES) {
      chooseMode(mode);
      expect(html(), mode).toContain("Preview in per-channel mode only");
    }
    chooseMode("linked");
    await extractAll();
    expect(mocks.extractBackgroundBatch).toHaveBeenCalledTimes(1);
    const markup = html();
    expect(markup).not.toContain("<img");
    for (const bin of BINS) expect(row(markup, bin)).toContain("37 samples, RMS 1.7180, 900ms");
    chooseMode("independent");
    expect(html()).not.toContain("Preview in per-channel mode only");
  });
});
