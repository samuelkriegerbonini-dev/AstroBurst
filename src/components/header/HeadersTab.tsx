import { useState, useCallback, useEffect, useRef, lazy, Suspense, memo } from "react";
import { Loader2 } from "lucide-react";
import { getFullHeader } from "../../services/header";
import { processFitsFull } from "../../services/fits";
import { fileStore } from "../../hooks/useFileStore";
import { useDisplayedImage, useFileContext, useNarrowbandContext, useRenderContext, useRgbContext } from "../../context/PreviewContext";
import { useCompositePreview } from "../../context/CompositeContext";
import { scientificInputRevisionOf } from "../../utils/pixelSource";
import { rgbViewOf } from "../../utils/rgbProduct";
import type { HeaderData, ProcessedFile } from "../../shared/types";

const HeaderExplorerPanel = lazy(() => import("./HeaderExplorerPanel"));
const HduSelectorPanel = lazy(() => import("./HduSelectorPanel"));

function HeadersTabInner() {
  const { file } = useFileContext();
  const { setRgbChannels } = useRgbContext();
  const { selectedPalette } = useNarrowbandContext();
  const { processed, processedVersion } = useRenderContext();
  const displayed = useDisplayedImage();
  const { compositePreviewUrl } = useCompositePreview();
  const composite = rgbViewOf({ compositePreviewUrl, fileIsRgb: !!file?.result?.is_rgb, filePath: file?.path ?? null, filePreviewUrl: file?.result?.previewUrl ?? null, processed })?.kind === "composite";
  const [selection, setSelection] = useState<{ fileId: string; kind: "displayed" | "source" } | null>(null);
  const selected = selection?.fileId === file?.id ? selection?.kind ?? "displayed" : "displayed";
  const hasAlternative = displayed.isProcessed || composite;
  const unavailableProduct = selected === "displayed" && (displayed.previewOnly || composite);
  const product = selected === "displayed" && displayed.isProcessed && !unavailableProduct;
  const targetPath = unavailableProduct ? null : product ? processed?.fitsPath ?? null : file?.path ?? null;
  const headerKey = JSON.stringify([selected, targetPath, processedVersion, scientificInputRevisionOf(file), selectedPalette, composite, displayed.previewOnly]);
  const [headerState, setHeaderState] = useState<{ key: string; data: HeaderData | null; loading: boolean; error: string | null } | null>(null);
  const [activation, setActivation] = useState<{ key: string; loading: boolean; error: string | null } | null>(null);
  const loadSeqRef = useRef(0);
  const activationSeqRef = useRef(0);
  const currentKeyRef = useRef<string | null>(headerKey);
  currentKeyRef.current = headerKey;
  const currentHeader = headerState?.key === headerKey ? headerState : null;
  const currentActivation = activation?.key === headerKey ? activation : null;
  const inspectedFile: ProcessedFile | null = !file || !targetPath ? null : product ? {
    ...file, path: targetPath, sourcePath: targetPath, imageRef: targetPath,
    name: targetPath.split(/[\\/]/).at(-1) ?? targetPath, result: null, startedAt: null, finishedAt: null,
  } : file;

  useEffect(() => {
    const loadSeq = loadSeqRef;
    const activationSeq = activationSeqRef;
    currentKeyRef.current = headerKey;
    return () => {
      currentKeyRef.current = null;
      ++loadSeq.current;
      ++activationSeq.current;
    };
  }, [headerKey]);

  const handleLoadHeader = useCallback(
    async (path: string) => {
      if (!targetPath || path !== targetPath) return;
      const seq = ++loadSeqRef.current;
      setHeaderState({ key: headerKey, data: null, loading: true, error: null });
      try {
        const data = await getFullHeader(path, selectedPalette);
        if (seq === loadSeqRef.current && currentKeyRef.current === headerKey) setHeaderState({ key: headerKey, data, loading: false, error: null });
      } catch (e) {
        if (seq !== loadSeqRef.current || currentKeyRef.current !== headerKey) return;
        setHeaderState({ key: headerKey, data: null, loading: false, error: e instanceof Error ? e.message : String(e) });
        throw e;
      }
    },
    [targetPath, headerKey, selectedPalette],
  );

  const handleAssignChannel = useCallback(
    (channel: string, path: string) => {
      const key = channel.toLowerCase();
      if (key !== "r" && key !== "g" && key !== "b") return;
      setRgbChannels((prev) => ({
        ...(prev ?? { r: null, g: null, b: null }),
        [key]: path,
      }));
    },
    [setRgbChannels],
  );

  const fileId = file?.id ?? null;
  const handleActivate = useCallback(
    async (ref: string) => {
      if (!fileId || product || unavailableProduct || currentKeyRef.current !== headerKey) return;
      const seq = ++activationSeqRef.current;
      setActivation({ key: headerKey, loading: true, error: null });
      try {
        const result = await processFitsFull(ref);
        if (seq !== activationSeqRef.current || currentKeyRef.current !== headerKey) return;
        fileStore.switchImageRef(fileId, ref, result);
      } catch (e) {
        if (seq !== activationSeqRef.current || currentKeyRef.current !== headerKey) return;
        setActivation({ key: headerKey, loading: false, error: e instanceof Error ? e.message : String(e) });
      } finally {
        if (seq === activationSeqRef.current && currentKeyRef.current === headerKey) setActivation((previous) => previous?.key === headerKey ? { ...previous, loading: false } : previous);
      }
    },
    [fileId, product, unavailableProduct, headerKey],
  );

  return (
    <Suspense
      fallback={
        <div className="flex items-center justify-center py-12">
          <Loader2 size={20} className="animate-spin text-zinc-500" />
        </div>
      }
    >
      <div className="flex flex-col gap-3 p-3">
        {file && hasAlternative && <div className="space-y-2">
          <div className="flex gap-2" role="group" aria-label="Header source">
            <button type="button" aria-pressed={selected === "displayed"} className={`rounded border px-2 py-1 text-[11px] ${selected === "displayed" ? "border-teal-600 text-teal-300" : "border-zinc-700 text-zinc-400"}`} onClick={() => setSelection({ fileId: file.id, kind: "displayed" })}>Displayed product</button>
            <button type="button" aria-pressed={selected === "source"} className={`rounded border px-2 py-1 text-[11px] ${selected === "source" ? "border-teal-600 text-teal-300" : "border-zinc-700 text-zinc-400"}`} onClick={() => setSelection({ fileId: file.id, kind: "source" })}>Source input</button>
          </div>
          <p className="break-all text-[10px] text-zinc-500">{selected === "source" ? `Source input: ${file.path}` : composite ? "Displayed composite: no FITS header is attached to this preview." : displayed.previewOnly ? "Displayed preview: no FITS header is attached to this image." : `Displayed product: ${targetPath}`}</p>
          {product && <p className="text-[10px] text-zinc-500">Inspect the product extensions here. Switch to Source input to change its displayed HDU.</p>}
        </div>}
        {currentHeader?.error && <p role="alert" className="text-[11px] text-red-400">Failed to read this header: {currentHeader.error}</p>}
        {file && targetPath && (
          <HduSelectorPanel
            key={`hdu-${headerKey}`}
            filePath={product ? targetPath : file.sourcePath || file.path}
            activePath={targetPath}
            activePlane={product ? null : file.result?.plane ?? null}
            onActivate={product ? undefined : handleActivate}
            activating={currentActivation?.loading ?? false}
            activateError={currentActivation?.error ?? null}
          />
        )}

        {inspectedFile && (
          <HeaderExplorerPanel
            key={`header-${headerKey}`}
            file={inspectedFile}
            onLoadHeader={handleLoadHeader}
            headerData={currentHeader?.data ?? null}
            isLoading={currentHeader?.loading ?? true}
            onAssignChannel={handleAssignChannel}
          />
        )}
      </div>
    </Suspense>
  );
}

export default memo(HeadersTabInner);
