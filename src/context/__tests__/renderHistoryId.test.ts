import { describe, expect, it } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { CompositeProvider } from "../CompositeContext";
import { PreviewProvider, fileKeyOf, getRenderRecord, useRenderActions } from "../PreviewContext";
import { withStep } from "../../utils/processingChain";
import type { ProcessedFile } from "../../shared/types";

const SOURCE = "C:/data/nrca1_i2d.fits";
const NODE = "C:/app/history/3f9c0a1e5b7d2c44/n0001.fits";

function fileOf(id: string): ProcessedFile {
  return {
    id,
    name: "nrca1_i2d.fits",
    path: SOURCE,
    sourcePath: SOURCE,
    imageRef: null,
    size: 1,
    status: "done",
    result: { png_path: "C:/out/a.png", previewUrl: "asset://a.png", dimensions: [8, 8], elapsed_ms: 1 },
    error: null,
    startedAt: null,
    finishedAt: null,
  };
}

function withActions(file: ProcessedFile | null, act: (actions: ReturnType<typeof useRenderActions>) => void): void {
  function Actor() {
    act(useRenderActions());
    return null;
  }
  const preview = createElement(PreviewProvider, { file, doneFiles: file ? [file] : [], children: createElement(Actor) });
  renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
}

describe("render records bound to a history", () => {
  it("setHistoryId binds the record and later publishes keep the binding", () => {
    const file = fileOf("bind");
    const key = fileKeyOf(file) as string;
    withActions(null, ({ publishProcessed, setHistoryId }) => {
      publishProcessed(key, { fitsPath: NODE, previewUrl: "asset://n0001.png", dimensions: [8, 8], label: "Background Extraction", kind: "processing", inputPath: SOURCE });
      setHistoryId(key, "3f9c0a1e5b7d2c44");
    });
    const bound = getRenderRecord(key);
    expect(bound?.historyId).toBe("3f9c0a1e5b7d2c44");
    expect(bound?.processed?.fitsPath).toBe(NODE);
    withActions(null, ({ setChain }) => setChain(key, (c) => withStep(c, "background", { fitsPath: NODE, previewUrl: null, dimensions: [8, 8] })));
    expect(getRenderRecord(key)?.historyId).toBe("3f9c0a1e5b7d2c44");
    expect(getRenderRecord(key)?.version).toBe(bound?.version);
  });

  it("resetProcessed clears the shown result and the chain but keeps the history binding", () => {
    const file = fileOf("reset");
    const key = fileKeyOf(file) as string;
    withActions(null, ({ publishProcessed, setHistoryId }) => {
      publishProcessed(key, { fitsPath: NODE, previewUrl: null, dimensions: [8, 8], label: "Background Extraction", kind: "processing", inputPath: SOURCE }, (c) =>
        withStep(c, "background", { fitsPath: NODE, previewUrl: null, dimensions: [8, 8] }),
      );
      setHistoryId(key, "3f9c0a1e5b7d2c44");
    });
    withActions(file, ({ resetProcessed }) => resetProcessed());
    const record = getRenderRecord(key);
    expect(record?.processed).toBeNull();
    expect(record?.chain.steps).toEqual({});
    expect(record?.historyId).toBe("3f9c0a1e5b7d2c44");
  });

  it("resetProcessed on a record with no history still forgets it", () => {
    const file = fileOf("plain");
    const key = fileKeyOf(file) as string;
    withActions(null, ({ publishProcessed }) => {
      publishProcessed(key, { fitsPath: "C:/out/x.fits", previewUrl: null, dimensions: [8, 8], label: "Solved WCS", kind: "processing", inputPath: SOURCE });
    });
    withActions(file, ({ resetProcessed }) => resetProcessed());
    expect(getRenderRecord(key)).toBeNull();
  });
});
