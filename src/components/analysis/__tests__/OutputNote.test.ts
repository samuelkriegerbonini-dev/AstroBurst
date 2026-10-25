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
});
