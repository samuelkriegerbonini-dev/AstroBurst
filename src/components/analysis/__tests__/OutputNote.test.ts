import { afterEach, describe, expect, it, vi } from "vitest";
import { createElement, isValidElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";

const reveal = vi.hoisted(() => vi.fn<(path: string) => Promise<string | null>>());

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../utils/revealInFolder", () => ({ revealInFolder: reveal }));

import { hooks } from "../../../test/hookRuntime";
import OutputNote from "../OutputNote";
import { OUTPUT_FOLDER_WARNING } from "../../../utils/outputNote";

const PATH = "C:\\Users\\me\\AppData\\Roaming\\com.astroburst.desktop\\output\\cube_moment0_5-39.fits";
const MANIFEST = "C:/out/moments-123/provenance.json";
const PROVENANCE = {
  source: { input_ref: "C:/data/cube.fits#hdu=1", sha256: "0123456789abcdef".repeat(4) },
  integrity: "sha256_before_open_and_before_publication",
  integrity_limit: "Checks do not create an immutable snapshot.",
};

describe("OutputNote", () => {
  it("shows the file name, the cleared-at-launch warning and Show in folder, and carries the full path", () => {
    const html = renderToStaticMarkup(createElement(OutputNote, { path: PATH }));
    expect(html).toContain("data-output-note");
    expect(html).toContain(`data-output-path="${PATH}"`);
    expect(html).toContain(">cube_moment0_5-39.fits<");
    expect(html).toContain(OUTPUT_FOLDER_WARNING.replace(">", "&gt;"));
    expect(html).toContain("Show in folder");
    expect(html).not.toContain('role="alert"');
  });

  it("shows the manifest and only the source identity and verification reported by the backend", () => {
    const html = renderToStaticMarkup(createElement(OutputNote, { path: MANIFEST, kind: "manifest", provenance: PROVENANCE }));
    expect(html).toContain("Show manifest");
    expect(html).toContain("Input SHA-256: 0123456789ab…");
    expect(html).toContain(PROVENANCE.source.sha256);
    expect(html).toContain("SHA-256 before reading and before publishing");
    expect(html).toContain(PROVENANCE.integrity_limit);
    expect(html).toContain("Keep the manifest with its result files");
    expect(html).not.toContain("Export &gt; FITS keeps a copy");
  });

  it("does not infer a content check from a manifest path or an unknown verification method", () => {
    const legacy = renderToStaticMarkup(createElement(OutputNote, { path: MANIFEST, kind: "manifest" }));
    expect(legacy).toContain("Show manifest");
    expect(legacy).not.toContain("SHA-256");
    const future = renderToStaticMarkup(createElement(OutputNote, { path: MANIFEST, kind: "manifest", provenance: { integrity: "future_method" } }));
    expect(future).toContain("future_method");
    expect(future).not.toContain("before publishing");
  });
});

type Props = Record<string, unknown> & { children?: ReactNode };

function elements(node: ReactNode): Props[] {
  if (Array.isArray(node)) return node.flatMap(elements);
  if (!isValidElement<Props>(node)) return [];
  return [node.props, ...elements(node.props.children)];
}

describe("OutputNote after Show in folder failed", () => {
  afterEach(() => hooks.mount());

  it("shows the failure under its own file only, not under the next output in the same slot", async () => {
    hooks.mount();
    reveal.mockResolvedValue("Could not open the folder C:\\out");
    const note = (path: string) => hooks.render((OutputNote as unknown as { type: (props: { path: string }) => unknown }).type, { path }) as ReactNode;
    const alertOf = (tree: ReactNode) => elements(tree).find((p) => p.role === "alert");
    (elements(note(PATH)).find((p) => p.type === "button")!.onClick as () => void)();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(alertOf(note(PATH))?.children).toBe("Could not open the folder C:\\out");
    expect(alertOf(note("C:\\out\\cube_moment1_5-39.fits"))).toBeUndefined();
  });

  it("reveals the current manifest path", async () => {
    hooks.mount();
    reveal.mockResolvedValue(null);
    const component = (OutputNote as unknown as { type: (props: Record<string, unknown>) => unknown }).type;
    const tree = hooks.render(component, { path: MANIFEST, kind: "manifest", provenance: PROVENANCE }) as ReactNode;
    (elements(tree).find((p) => p.type === "button")!.onClick as () => void)();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(reveal).toHaveBeenLastCalledWith(MANIFEST);
  });
});
