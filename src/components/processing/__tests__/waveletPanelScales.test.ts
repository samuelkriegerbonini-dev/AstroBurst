import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

const { captured, clicks, denoiseMock, clickDuringRender } = vi.hoisted(() => {
  const clicks = new Set<string>();
  type Factory = (type: unknown, props: Record<string, unknown> | null, ...rest: unknown[]) => unknown;
  const clickDuringRender = (factory: Factory): Factory => (type, props, ...rest) => {
    const id = props?.["data-testid"];
    if (typeof type === "string" && typeof id === "string" && clicks.delete(id)) ((props?.onClick ?? props?.onChange) as () => void)();
    return factory(type, props, ...rest);
  };
  return {
    captured: { onClick: null as null | (() => void) },
    clicks,
    denoiseMock: vi.fn(),
    clickDuringRender,
  };
});

vi.mock("react/jsx-runtime", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react/jsx-runtime")>();
  return { ...actual, jsx: clickDuringRender(actual.jsx as never), jsxs: clickDuringRender(actual.jsxs as never) };
});

vi.mock("react/jsx-dev-runtime", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react/jsx-dev-runtime")>();
  return { ...actual, jsxDEV: clickDuringRender(actual.jsxDEV as never) };
});

vi.mock("../../ui", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../ui")>();
  return {
    ...actual,
    RunButton: (props: { onClick: () => void }) => {
      captured.onClick = props.onClick;
      return null;
    },
  };
});

vi.mock("../../../services/processing", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/processing")>()),
  recordWaveletDenoise: denoiseMock,
}));

import { CompositeProvider } from "../../../context/CompositeContext";
import { PreviewProvider, fileKeyOf } from "../../../context/PreviewContext";
import { historyRerunStore } from "../../../utils/historyRerunStore";
import { waveletOp } from "../../../services/processing";
import WaveletPanel from "../WaveletPanel";
import type { ProcessedFile } from "../../../shared/types";

const PATH = "C:/data/673nmos.fits";
const SCOPE = { imageKey: PATH, basePath: null };
const THRESHOLDING_HINT =
  "Soft shrinks every coefficient toward zero (smoother); Hard keeps coefficients above the threshold unchanged (sharper, can leave speckles).";

function fileOf(id: string): ProcessedFile {
  return {
    id,
    name: "673nmos.fits",
    path: PATH,
    sourcePath: PATH,
    imageRef: null,
    size: 1,
    status: "done",
    result: { png_path: "C:/out/673nmos.png", previewUrl: "asset://C:/out/673nmos.png", dimensions: [1600, 1600], elapsed_ms: 1, is_rgb: false },
    error: null,
    startedAt: null,
    finishedAt: null,
  };
}

function render(file: ProcessedFile): string {
  const panel = createElement(WaveletPanel, {
    selectedFile: file,
    outputDir: "C:/out",
    fileKey: fileKeyOf(file),
    compositeMode: false,
    compositeInput: null,
    onCompositeDone: () => {},
    fileName: file.name,
  });
  const preview = createElement(PreviewProvider, { file, doneFiles: [file], children: panel });
  return renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
}

function between(html: string, start: string, end: string): string {
  const from = html.indexOf(start);
  const to = from < 0 ? -1 : html.indexOf(end, from + start.length);
  if (to < 0) throw new Error(`no markup from "${start}" to "${end}"`);
  return html.slice(from, to);
}

function texts(html: string): string[] {
  return html
    .replace(/<!-- -->/g, "")
    .split(/<[^>]*>/)
    .map((t) => t.trim())
    .filter((t) => t.length > 0);
}

function rangeNames(html: string): string[] {
  return [...html.matchAll(/<input[^>]*type="range"[^>]*>/g)].map((m) => /aria-label="([^"]*)"/.exec(m[0])?.[1] ?? "");
}

function radio(html: string, value: string): string {
  const tag = [...html.matchAll(/<input[^>]*type="radio"[^>]*>/g)].map((m) => m[0]).find((t) => t.includes(`value="${value}"`));
  if (!tag) throw new Error(`no ${value} radio`);
  return tag;
}

function textOfId(html: string, id: string | undefined): string {
  if (!id) return "";
  const escaped = id.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = new RegExp(`<([a-z]+)[^>]*\\sid="${escaped}"[^>]*>([\\s\\S]*?)</\\1>`).exec(html);
  return match ? texts(match[2]).join(" ") : "";
}

beforeEach(() => {
  clicks.clear();
  captured.onClick = null;
  denoiseMock.mockReset();
  historyRerunStore.clear();
});

describe("Denoise scales", () => {
  it("names each scale by its pixel range and shows each threshold once in its row", () => {
    const html = render(fileOf("wd-scales"));
    const thresholds = between(html, "Threshold per scale (sigma)", "Detail bias per scale");
    expect(texts(thresholds).filter((t) => t === "3.0")).toHaveLength(1);
    expect(rangeNames(thresholds)).toEqual([
      "Fine (1-2 px) threshold",
      "Small (2-4 px) threshold",
      "Medium (4-8 px) threshold",
      "Large (8-16 px) threshold",
      "Very large (16-32 px) threshold",
    ]);
    expect(texts(thresholds)).toContain("Fine (1-2 px)");
    expect(texts(thresholds)).toContain("A threshold of 0 leaves its scale untouched.");
    expect(texts(thresholds).join(" ")).toContain("FWHM");
  });

  it("shows each detail bias once in its row, under the same scale names", () => {
    const html = render(fileOf("wd-bias"));
    const bias = between(html, "Detail bias per scale", "0 keeps the layer");
    expect(texts(bias).filter((t) => t === "0.00")).toHaveLength(5);
    expect(rangeNames(bias)[0]).toBe("Fine (1-2 px) bias");
  });

  it("names the scales after the fifth as Scale n with their range", () => {
    historyRerunStore.request({
      section: "denoise",
      op: waveletOp({ numScales: 8, thresholds: [3, 2.5, 2, 1.5, 1, 1, 1, 1], linear: false }),
      node: "n0002",
      downstream: false,
      scope: SCOPE,
    });
    const thresholds = between(render(fileOf("wd-eight")), "Threshold per scale (sigma)", "Detail bias per scale");
    expect(rangeNames(thresholds).slice(5)).toEqual(["Scale 6 (32-64 px) threshold", "Scale 7 (64-128 px) threshold", "Scale 8 (128-256 px) threshold"]);
  });
});

describe("Denoise thresholding", () => {
  it("offers Soft and Hard as a labelled radiogroup with Soft checked and says what each does", () => {
    const html = render(fileOf("wd-soft"));
    const group = /<div[^>]*role="radiogroup"[^>]*>/.exec(html)?.[0] ?? "";
    expect(textOfId(html, /aria-labelledby="([^"]*)"/.exec(group)?.[1])).toBe("Thresholding");
    expect(textOfId(html, /aria-describedby="([^"]*)"/.exec(group)?.[1])).toBe(THRESHOLDING_HINT);
    expect(texts(/<div[^>]*role="radiogroup"[^>]*>([\s\S]*?)<\/div>/.exec(html)?.[1] ?? "")).toEqual(["Soft", "Hard"]);
    expect(radio(html, "soft")).toContain('checked=""');
    expect(radio(html, "hard")).not.toContain("checked");
    expect(/name="([^"]*)"/.exec(radio(html, "soft"))?.[1]).toBe(/name="([^"]*)"/.exec(radio(html, "hard"))?.[1]);
    expect(html).not.toContain("Soft threshold (linear)");
  });

  it("checks Hard when Re-run with opens a hard-threshold step", () => {
    historyRerunStore.request({ section: "denoise", op: waveletOp({ linear: false }), node: "n0002", downstream: false, scope: SCOPE });
    const html = render(fileOf("wd-rerun-hard"));
    expect(radio(html, "hard")).toContain('checked=""');
    expect(radio(html, "soft")).not.toContain("checked");
  });

  it("runs a hard threshold once Hard is picked", async () => {
    denoiseMock.mockResolvedValue({ previewUrl: "asset://C:/out/673nmos_denoised.png", fits_path: "C:/out/673nmos_denoised.fits", node: "n0001" });
    clicks.add("wavelet-thresholding-hard");
    const html = render(fileOf("wd-run-hard"));
    expect(radio(html, "hard")).toContain('checked=""');
    if (!captured.onClick) throw new Error("RunButton not rendered");
    captured.onClick();
    await vi.waitFor(() => expect(denoiseMock).toHaveBeenCalledTimes(1));
    expect(denoiseMock.mock.calls[0][1]).toBe(PATH);
    expect(denoiseMock.mock.calls[0][2]).toMatchObject({ numScales: 5, linear: false });
  });
});
