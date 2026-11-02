import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactElement, type ReactNode } from "react";
import type { HeaderData, ProcessedFile, ProcessResult } from "../../../shared/types";
import type { ProcessedResult } from "../../../shared/types/preview";

const mocks = vi.hoisted(() => ({ header: vi.fn(), process: vi.fn(), switchRef: vi.fn(), assign: vi.fn() }));
vi.mock("react", async (original) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await original()));
vi.mock("../../../services/header", () => ({ getFullHeader: mocks.header }));
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
import { hooks } from "../../../test/hookRuntime";

type Props = Record<string, unknown> & { children?: ReactNode };
const component = (HeadersTab as unknown as { type: (props: object) => ReactNode }).type;
const render = () => hooks.render(component, {}) as ReactNode;
function find(node: ReactNode, match: (node: ReactElement<Props>) => boolean): ReactElement<Props> | undefined {
  if (Array.isArray(node)) return node.map((child) => find(child, match)).find(Boolean);
  if (!isValidElement<Props>(node)) return undefined;
  return match(node) ? node : find(node.props.children, match);
}
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
});
