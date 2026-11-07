import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { HeaderData, ProcessedFile, ProcessResult } from "../../../shared/types";
import type { ProcessedResult } from "../../../shared/types/preview";
import type { FitsExtension } from "../../../services/header";

const mocks = vi.hoisted(() => ({ header: vi.fn(), extensions: vi.fn(), process: vi.fn(), switchRef: vi.fn(), assign: vi.fn() }));
vi.mock("react", async (original) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await original()));
vi.mock("../../../services/header", () => ({ getFullHeader: mocks.header, getFitsExtensions: mocks.extensions, getHeaderByHdu: vi.fn() }));
vi.mock("../../../services/fits", () => ({ processFitsFull: mocks.process }));
vi.mock("../../../hooks/useFileStore", () => ({ fileStore: { switchImageRef: mocks.switchRef } }));

const source: ProcessedFile = {
  id: "one", path: "/data/source.fits#hdu=1", sourcePath: "/data/source.fits", name: "source.fits", imageRef: "/data/source.fits#hdu=1", reloads: 0, finishedAt: 1, size: 100, status: "done", error: null, startedAt: 0,
  result: { header: { BUNIT: "MJy/sr" }, png_path: "source.png", dimensions: [4, 5], elapsed_ms: 1, is_rgb: false, previewUrl: "source.png", plane: { kind: "hdu", index: 1, key: null, extname: "SCI", extver: 1, is_dq: false, is_err: false, dq_ref: null, err_ref: null, source_path: "/data/source.fits", dq_table: null } },
};
const view = { file: source, processed: { fitsPath: "/out/velocity.fits", label: "Velocity", previewUrl: "out.png", dimensions: [4, 5] } as ProcessedResult | null, version: 1, palette: "SHO", composite: null as string | null };
vi.mock("../../../context/PreviewContext", () => ({
  useFileContext: () => ({ file: view.file }),
  useRgbContext: () => ({ setRgbChannels: mocks.assign }),
  useNarrowbandContext: () => ({ selectedPalette: view.palette }),
  useRenderContext: () => ({ processed: view.processed, processedVersion: view.version }),
  useDisplayedImage: () => ({ path: view.processed?.fitsPath ?? view.file.path, isProcessed: view.processed !== null, previewOnly: view.processed !== null && view.processed.fitsPath === null, label: view.processed?.label }),
}));
vi.mock("../../../context/CompositeContext", () => ({ useCompositePreview: () => ({ compositePreviewUrl: view.composite }) }));

import HeadersTab from "../HeadersTab";
import HduSelectorPanel from "../HduSelectorPanel";
import { hooks } from "../../../test/hookRuntime";

type Props = Record<string, unknown> & { children?: ReactNode };
const component = (HeadersTab as unknown as { type: (props: object) => ReactNode }).type;
const render = () => hooks.render(component, {}) as ReactNode;
function find(node: ReactNode, match: (node: ReactElement<Props>) => boolean): ReactElement<Props> | undefined {
  if (Array.isArray(node)) return node.map((child) => find(child, match)).find(Boolean);
  if (!isValidElement<Props>(node)) return undefined;
  return match(node) ? node : find(node.props.children, match);
}
function findAll(node: ReactNode, match: (node: ReactElement<Props>) => boolean): ReactElement<Props>[] {
  if (Array.isArray(node)) return node.flatMap((child) => findAll(child, match));
  if (!isValidElement<Props>(node)) return [];
  return [...(match(node) ? [node] : []), ...findAll(node.props.children, match)];
}
const text = (node: ReactNode) => renderToStaticMarkup(node as ReactElement).replace(/<[^>]*>/g, "");
const explorer = (tree: ReactNode) => find(tree, (node) => "onLoadHeader" in node.props)!;
const hdu = (tree: ReactNode) => find(tree, (node) => "filePath" in node.props && "activePath" in node.props)!;
const load = (tree: ReactNode) => (explorer(tree).props.onLoadHeader as (path: string) => Promise<void>)((explorer(tree).props.file as ProcessedFile).path);
function clickSource() { (find(render(), (node) => node.type === "button" && node.props.children === "Source input")!.props.onClick as () => void)(); }
const data = (path: string, unit: string): HeaderData => ({ file_path: path, file_name: path, total_cards: 1, cards: [{ key: "BUNIT", value: unit }], categories: {}, filter_detection: null, filename_hint: null });
function deferred<T>() { let resolve!: (value: T) => void; let reject!: (reason: unknown) => void; const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; }

beforeEach(() => {
  hooks.mount();
  vi.clearAllMocks();
  view.file = { ...source, result: source.result };
  view.processed = { fitsPath: "/out/velocity.fits", label: "Velocity", previewUrl: "out.png", dimensions: [4, 5] } as ProcessedResult;
  view.version = 1;
  view.palette = "SHO";
  view.composite = null;
  mocks.header.mockImplementation(async (path: string) => data(path, "km/s"));
});
afterEach(() => hooks.mount());

describe("HeadersTab active product and source", () => {
  it("defaults to the derived FITS and does not activate a source HDU from the product inspector", async () => {
    const tree = render();
    expect((explorer(tree).props.file as ProcessedFile).path).toBe("/out/velocity.fits");
    expect((explorer(tree).props.file as ProcessedFile).result).toBeNull();
    expect(hdu(tree).props.filePath).toBe("/out/velocity.fits");
    expect(hdu(tree).props.activePlane).toBeNull();
    expect(hdu(tree).props.onActivate).toBeUndefined();
    await load(tree);
    expect(mocks.header).toHaveBeenCalledWith("/out/velocity.fits", "SHO");
    expect((explorer(render()).props.headerData as HeaderData).cards[0].value).toBe("km/s");
  });

  it("allows explicit source inspection and keeps source plane activation compatible", async () => {
    render();
    clickSource();
    const tree = render();
    expect((explorer(tree).props.file as ProcessedFile).path).toBe(source.path);
    expect(hdu(tree).props.filePath).toBe(source.sourcePath);
    expect(hdu(tree).props.activePlane).toEqual(source.result?.plane);
    const processResult = { dimensions: [2, 3] } as ProcessResult;
    mocks.process.mockResolvedValue(processResult);
    await (hdu(tree).props.onActivate as (path: string) => Promise<void>)("/data/source.fits#hdu=2");
    expect(mocks.switchRef).toHaveBeenCalledWith("one", "/data/source.fits#hdu=2", processResult);
  });

  it("hides the old header synchronously on product revision and reloads the HDU inspector", async () => {
    const first = render();
    await load(first);
    expect(explorer(render()).props.headerData).not.toBeNull();
    view.version++;
    const changed = render();
    expect(explorer(changed).props.headerData).toBeNull();
    expect(explorer(changed).key).not.toBe(explorer(first).key);
    expect(hdu(changed).key).not.toBe(hdu(first).key);
    await load(changed);
    expect(mocks.header).toHaveBeenCalledTimes(2);
  });

  it("invalidates headers when the same source path is reloaded", async () => {
    view.processed = null;
    const first = render();
    await load(first);
    view.file = { ...view.file, reloads: 1, finishedAt: 2 };
    const changed = render();
    expect(explorer(changed).props.headerData).toBeNull();
    expect(hdu(changed).key).not.toBe(hdu(first).key);
  });

  it("discards a late product response after selecting the source", async () => {
    const pending = deferred<HeaderData>();
    mocks.header.mockReturnValueOnce(pending.promise);
    const old = load(render());
    clickSource();
    const changed = render();
    await load(changed);
    pending.resolve(data("/out/velocity.fits", "OLD"));
    await old;
    expect((explorer(render()).props.headerData as HeaderData).file_path).toBe(source.path);
  });

  it("does not treat a PNG or composite preview as a FITS header", () => {
    view.processed = { ...view.processed!, fitsPath: null };
    let tree = render();
    expect(explorer(tree)).toBeUndefined();
    expect(hdu(tree)).toBeUndefined();
    clickSource();
    expect(explorer(render())).toBeDefined();
    hooks.mount();
    view.processed = null;
    view.composite = "composite.png";
    tree = render();
    expect(explorer(tree)).toBeUndefined();
    clickSource();
    expect(explorer(render())).toBeDefined();
  });

  it("invalidates a pending HDU activation when a different file is selected", async () => {
    view.processed = null;
    const pending = deferred<ProcessResult>();
    mocks.process.mockReturnValue(pending.promise);
    const activation = (hdu(render()).props.onActivate as (path: string) => Promise<void>)("/data/source.fits#hdu=2");
    view.file = { ...view.file, id: "two", path: "/data/other.fits" };
    render();
    pending.resolve({} as ProcessResult);
    await activation;
    expect(mocks.switchRef).not.toHaveBeenCalled();
  });

  it("invalidates palette-dependent detection and ignores responses after unmount", async () => {
    await load(render());
    view.palette = "HOO";
    const changed = render();
    expect(explorer(changed).props.headerData).toBeNull();
    const pending = deferred<HeaderData>();
    mocks.header.mockReturnValue(pending.promise);
    const old = load(changed);
    hooks.mount();
    pending.resolve(data("/out/velocity.fits", "OLD"));
    await old;
    expect(explorer(render()).props.headerData).toBeNull();
  });

  it("keeps the source inspector and its header while products are published", async () => {
    render();
    clickSource();
    const first = render();
    await load(first);
    view.version++;
    const republished = render();
    view.version++;
    view.processed = { ...view.processed!, fitsPath: null };
    const pngFrame = render();
    for (const tree of [republished, pngFrame]) {
      expect(explorer(tree).key).toBe(explorer(first).key);
      expect(hdu(tree).key).toBe(hdu(first).key);
      expect((explorer(tree).props.headerData as HeaderData).file_path).toBe(source.path);
    }
    expect(mocks.header).toHaveBeenCalledTimes(1);
  });

  it("reports a failed header read in one error block and retries it from there", async () => {
    mocks.header.mockRejectedValueOnce(new Error("Header is unreadable"));
    await expect(load(render())).resolves.toBeUndefined();
    const failed = render();
    const alerts = findAll(failed, (node) => node.props.role === "alert");
    expect(alerts).toHaveLength(1);
    expect(text(alerts[0])).toContain("Header is unreadable");
    const retry = find(failed, (node) => node.type === "button" && text(node) === "Retry");
    expect(retry).toBeDefined();
    await (retry!.props.onClick as () => Promise<void>)();
    expect(mocks.header).toHaveBeenCalledTimes(2);
    const recovered = render();
    expect(findAll(recovered, (node) => node.props.role === "alert")).toHaveLength(0);
    expect((explorer(recovered).props.headerData as HeaderData).cards[0].value).toBe("km/s");
  });

  it("keeps the failed-read row and its Retry button mounted and busy while the header is read again, then announces a repeated failure", async () => {
    mocks.header.mockRejectedValueOnce(new Error("Header is unreadable"));
    await load(render());
    const pending = deferred<HeaderData>();
    mocks.header.mockReturnValueOnce(pending.promise);
    const retryOf = (tree: ReactNode) => find(tree, (node) => node.type === "button" && text(node).startsWith("Retry"));
    const alertOf = (tree: ReactNode) => find(tree, (node) => node.props.role === "alert");
    const failed = render();
    const firstAlert = alertOf(failed)!;
    const retrying = (retryOf(failed)!.props.onClick as () => Promise<void>)();
    const busy = render();
    const busyRetry = retryOf(busy);
    expect(busyRetry, "Retry stays mounted while the header is read again").toBeDefined();
    expect(busyRetry!.props["aria-busy"]).toBe(true);
    expect(busyRetry!.props["aria-disabled"]).toBe(true);
    expect(busyRetry!.props.disabled).toBeFalsy();
    (busyRetry!.props.onClick as (() => unknown) | undefined)?.();
    expect(mocks.header).toHaveBeenCalledTimes(2);
    expect(text(alertOf(busy)!)).toContain("Header is unreadable");
    expect(alertOf(busy)!.key).toBe(firstAlert.key);
    pending.reject(new Error("Header is still unreadable"));
    await retrying;
    const failedAgain = render();
    expect(findAll(failedAgain, (node) => node.props.role === "alert")).toHaveLength(1);
    expect(text(alertOf(failedAgain)!)).toContain("Header is still unreadable");
    expect(alertOf(failedAgain)!.key).not.toBe(firstAlert.key);
    expect(retryOf(failedAgain)!.props["aria-busy"]).toBeFalsy();
    expect(retryOf(failedAgain)!.props["aria-disabled"]).toBeFalsy();
  });

  it("marks the source plane as the source of the displayed product in source mode", () => {
    expect(hdu(render()).props.sourceOfProduct).toBeFalsy();
    clickSource();
    expect(hdu(render()).props.sourceOfProduct).toBe(true);
    hooks.mount();
    view.processed = null;
    expect(hdu(render()).props.sourceOfProduct).toBeFalsy();
  });
});

describe("HduSelectorPanel shown plane", () => {
  const extension = (index: number, extname: string): FitsExtension => ({
    index, extname, naxis: 2, naxis1: 4, naxis2: 5, naxis3: 0, bitpix: -32, has_data: index > 0, extver: 1, ref: `/data/source.fits#hdu=${index}`, kind: "hdu", is_dq: false, is_err: false,
  });
  const panel = (sourceOfProduct?: boolean) => renderToStaticMarkup(hooks.render(HduSelectorPanel, {
    filePath: "/data/source.fits", activePath: "/data/source.fits#hdu=1", activePlane: source.result?.plane, sourceOfProduct,
  }) as ReactElement);

  async function loaded(sourceOfProduct?: boolean): Promise<string> {
    panel(sourceOfProduct);
    for (let tick = 0; tick < 5; tick++) await Promise.resolve();
    return panel(sourceOfProduct);
  }

  beforeEach(() => {
    mocks.extensions.mockResolvedValue({ extensions: [extension(0, "PRIMARY"), extension(1, "SCI"), extension(2, "ERR")] });
  });

  it("calls the active plane the displayed plane when the source is on screen", async () => {
    const html = await loaded();
    expect(html).toContain('title="Currently displayed plane"');
    expect(html).not.toContain("Source plane of the displayed product");
  });

  it("calls the active plane the source of the displayed product while a product is on screen", async () => {
    const html = await loaded(true);
    expect(html).toContain('title="Source plane of the displayed product"');
    expect(html).not.toContain("Currently displayed plane");
  });
});
