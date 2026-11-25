import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { prerender } from "react-dom/static";

vi.mock("../../../context/ComposeWizardContext", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../context/ComposeWizardContext")>()),
  useComposeWizardContext: () => {
    throw new Error("ProcessingTab subscribed to the whole wizard context");
  },
}));

import { CompositeProvider } from "../../../context/CompositeContext";
import { ComposeWizardProvider } from "../../../context/ComposeWizardContext";
import { PreviewProvider } from "../../../context/PreviewContext";
import ProcessingTab from "../ProcessingTab";
import type { ProcessedFile } from "../../../shared/types";

const FILE: ProcessedFile = {
  id: "subscription",
  name: "m31_L.fits",
  path: "C:/data/m31_L.fits",
  sourcePath: "C:/data/m31_L.fits",
  imageRef: null,
  size: 1,
  status: "done",
  result: { png_path: "C:/out/m31_L.png", previewUrl: "asset://C:/out/m31_L.png", dimensions: [64, 64], elapsed_ms: 1, is_rgb: false },
  error: null,
  startedAt: null,
  finishedAt: null,
};

describe("ProcessingTab and the wizard", () => {
  it("reads only the composite-ready flag and the wizard dispatch, so a wizard edit does not re-render its panels", async () => {
    const tree = createElement(CompositeProvider, {
      children: createElement(ComposeWizardProvider, {
        children: createElement(PreviewProvider, { file: FILE, doneFiles: [FILE], children: createElement(ProcessingTab) }),
      }),
    });
    const errors: unknown[] = [];
    const { prelude } = await prerender(tree, { onError: (e) => { errors.push(e); } });
    const html = await new Response(prelude).text();
    expect(errors.map(String)).toEqual([]);
    expect(html).toContain("Extract Background");
  });
});
