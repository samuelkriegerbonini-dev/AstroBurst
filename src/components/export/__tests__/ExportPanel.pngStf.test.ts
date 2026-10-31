import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createElement, isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";

const FILE = "C:\\data\\673nmos.fits";
const CHANNELS = { r: "C:\\data\\673nmos.fits", g: "C:\\data\\656nmos.fits", b: "C:\\data\\502nmos.fits" };

const exportPngMock = vi.hoisted(() => vi.fn<(...args: unknown[]) => Promise<unknown>>());
const exportRgbPngMock = vi.hoisted(() => vi.fn<(...args: unknown[]) => Promise<unknown>>());

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../hooks/useRegionKey", () => ({ useRegionKey: () => FILE }));
vi.mock("../../../hooks/useRegionStore", () => ({ useRegionDoc: () => ({ regions: [], selectedId: null }) }));
vi.mock("../../../services/astrometry", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/astrometry")>()),
  getWcsInfo: async () => ({ pixel_scale_arcsec: 0 }),
}));
vi.mock("../../../infrastructure/tauri", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../infrastructure/tauri")>()),
  getExportDir: async () => "C:/out",
}));
vi.mock("../../../services/export", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/export")>()),
  exportPng: exportPngMock,
  exportRgbPng: exportRgbPngMock,
}));

import { hooks } from "../../../test/hookRuntime";
import ExportPanel from "../ExportPanel";
import { DEFAULT_DISPLAY_SETTINGS, type DisplaySettings } from "../../../shared/types/display";

const AUTO = { shadow: 0.0043, midtone: 0.0085, highlight: 1 };

const PROPS = {
  filePath: FILE,
  exportPath: FILE,
  stfParams: AUTO,
  autoStf: AUTO,
  display: DEFAULT_DISPLAY_SETTINGS as DisplaySettings,
  pngSourceRgb: false,
  onExport: () => Promise.resolve(),
  rgbChannels: null as typeof CHANNELS | null,
  headerChannels: null as typeof CHANNELS | null,
};

type Props = typeof PROPS;
type AnyProps = Record<string, unknown> & { children?: ReactNode };

const panel = ExportPanel as unknown as (props: Props) => ReactNode;
const render = (props: Partial<Props> = {}) => hooks.render(panel, { ...PROPS, ...props }) as ReactNode;
const markup = (props: Partial<Props> = {}) => renderToStaticMarkup(createElement(panel, { ...PROPS, ...props }));

function elements(node: ReactNode): ReactElement<AnyProps>[] {
  if (Array.isArray(node)) return node.flatMap(elements);
  if (!isValidElement<AnyProps>(node)) return [];
  return [node, ...elements(node.props.children)];
}

function textOf(node: ReactNode): string {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  return isValidElement<AnyProps>(node) ? textOf(node.props.children) : "";
}

function switchTag(html: string, label: string): string {
  const tag = html.match(new RegExp(`<button[^>]*aria-label="${label}"[^>]*>`))?.[0];
  if (!tag) throw new Error(`no switch named "${label}" in the rendered panel`);
  return tag;
}

const attr = (tag: string, name: string) => tag.match(new RegExp(`${name}="([^"]*)"`))?.[1] ?? null;

const PNG_SWITCH = "Apply STF stretch";
const FITS_SWITCH = "Apply current STF stretch";

function pngToggle(tree: ReactNode): ReactElement<AnyProps> {
  const toggle = elements(tree).find((el) => el.props.label === PNG_SWITCH);
  if (!toggle) throw new Error(`no Toggle labelled "${PNG_SWITCH}"`);
  return toggle;
}

function pngNote(tree: ReactNode): ReactElement<AnyProps> | undefined {
  const id = pngToggle(tree).props.describedBy;
  return elements(tree).find((el) => typeof id === "string" && el.props.id === id);
}

const mismatchOf = (tree: ReactNode) => elements(tree).find((el) => "data-png-stf-mismatch" in el.props);

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("ExportPanel PNG switch (EX-01)", () => {
  beforeEach(() => {
    hooks.mount();
    exportPngMock.mockReset().mockResolvedValue({});
    exportRgbPngMock.mockReset().mockResolvedValue({});
  });

  afterEach(() => hooks.mount());

  it("starts the PNG switch on as 'Apply STF stretch' and leaves the FITS switch off", () => {
    const html = markup();
    expect(attr(switchTag(html, PNG_SWITCH), "aria-checked")).toBe("true");
    expect(attr(switchTag(html, FITS_SWITCH), "aria-checked")).toBe("false");
  });

  it("describes the PNG switch with a note that names the Image tool's STF state", () => {
    const html = markup();
    const noteId = attr(switchTag(html, PNG_SWITCH), "aria-describedby");
    expect(noteId).toBeTruthy();
    expect(html).toContain(`id="${noteId}"`);
    expect(textOf(pngNote(render()))).toContain("Mono: the Image tool's STF (Auto STF).");
    expect(textOf(pngNote(render({ stfParams: { shadow: 0, midtone: 0.5, highlight: 1 } })))).toContain("(Linear)");
    expect(textOf(pngNote(render({ stfParams: { ...AUTO, midtone: 0.0121 } })))).toContain("(Manual STF)");
  });

  it("adds the amber line while the viewer shows another stretch and the switch is on, and drops it when the switch is off", () => {
    expect(mismatchOf(render())).toBeUndefined();
    const asinh = { display: { ...DEFAULT_DISPLAY_SETTINGS, stretch: "asinh" as const } };
    const tree = render(asinh);
    expect(textOf(mismatchOf(tree))).toBe("The viewer shows the asinh stretch; the PNG uses the MTF STF instead.");
    expect(textOf(pngNote(tree))).toContain("asinh");
    (pngToggle(tree).props.onChange as (v: boolean) => void)(false);
    expect(mismatchOf(render(asinh))).toBeUndefined();
  });

  it("exports a mono PNG with the Image tool's STF by default", async () => {
    const run = elements(render()).find((el) => el.props.label === "Export as PNG");
    await (run?.props.onClick as () => Promise<void>)();
    expect(exportPngMock).toHaveBeenCalledTimes(1);
    const [path, output, options] = exportPngMock.mock.calls[0];
    expect(path).toBe(FILE);
    expect(String(output)).toMatch(/^C:\/out\/673nmos_stf_\d{8}-\d{6}-\d{3}\.png$/);
    expect(options).toEqual({ bitDepth: 16, applyStfStretch: true, ...AUTO });
  });

  it("keeps an RGB image on its own PNG path, as shown or with the automatic linked STF, instead of the Image tool's mono STF", async () => {
    const tree = render({ pngSourceRgb: true, display: { ...DEFAULT_DISPLAY_SETTINGS, stretch: "asinh" } });
    expect(mismatchOf(tree), "the mono display stretch says nothing about an RGB export").toBeUndefined();
    const run = elements(tree).find((el) => el.props.label === "Export as PNG");
    await (run?.props.onClick as () => Promise<void>)();
    expect(exportPngMock).toHaveBeenCalledTimes(1);
    const [, output, options] = exportPngMock.mock.calls[0];
    expect(String(output)).toMatch(/^C:\/out\/673nmos_\d{8}-\d{6}-\d{3}\.png$/);
    expect(options).toMatchObject({ bitDepth: 16, applyStfStretch: false });
  });

  it("exports Headers-tab RGB channels with the switch on by default, so the backend applies its automatic linked STF", async () => {
    const tree = render({ rgbChannels: CHANNELS, headerChannels: CHANNELS });
    const button = elements(tree).find((el) => el.type === "button" && textOf(el).includes("Export RGB as PNG"));
    await (button?.props.onClick as () => Promise<void>)();
    await flush();
    expect(exportRgbPngMock).toHaveBeenCalledTimes(1);
    const [r, g, b, output, options] = exportRgbPngMock.mock.calls[0];
    expect([r, g, b]).toEqual([CHANNELS.r, CHANNELS.g, CHANNELS.b]);
    expect(String(output)).toMatch(/^C:\/out\/astroburst_composite_stf_16bit_\d{8}-\d{6}-\d{3}\.png$/);
    expect(options).toEqual({ bitDepth: 16, applyStfStretch: true, linked: true });
  });
});
