import { useState, useCallback, lazy, Suspense, memo } from "react";
import { Loader2 } from "lucide-react";
import { getFullHeader } from "../../services/header";
import { processFitsFull } from "../../services/fits";
import { fileStore } from "../../hooks/useFileStore";
import { useFileContext, useRgbContext } from "../../context/PreviewContext";
import type { HeaderData } from "../../shared/types";

const HeaderExplorerPanel = lazy(() => import("./HeaderExplorerPanel"));
const HduSelectorPanel = lazy(() => import("./HduSelectorPanel"));

function HeadersTabInner() {
  const { file } = useFileContext();
  const { setRgbChannels } = useRgbContext();

  const [headerData, setHeaderData] = useState<HeaderData | null>(null);
  const [headerLoading, setHeaderLoading] = useState(false);
  const [activating, setActivating] = useState(false);
  const [activateError, setActivateError] = useState<string | null>(null);

  const handleLoadHeader = useCallback(
    async (path: string) => {
      setHeaderLoading(true);
      setHeaderData(null);
      try {
        const data = await getFullHeader(path);
        setHeaderData(data);
      } catch (e) {
        console.error("Header load failed:", e);
        throw e;
      } finally {
        setHeaderLoading(false);
      }
    },
    [],
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
      if (!fileId) return;
      setActivating(true);
      setActivateError(null);
      try {
        const result = await processFitsFull(ref);
        fileStore.switchImageRef(fileId, ref, result);
      } catch (e) {
        console.error("Plane switch failed:", e);
        setActivateError(e instanceof Error ? e.message : String(e));
      } finally {
        setActivating(false);
      }
    },
    [fileId],
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
        {file?.path && (
          <HduSelectorPanel
            filePath={file.sourcePath || file.path}
            activePath={file.path}
            activePlane={file.result?.plane ?? null}
            onActivate={handleActivate}
            activating={activating}
            activateError={activateError}
          />
        )}

        {file && (
          <HeaderExplorerPanel
            file={file}
            onLoadHeader={handleLoadHeader}
            headerData={headerData}
            isLoading={headerLoading}
            onAssignChannel={handleAssignChannel}
          />
        )}
      </div>
    </Suspense>
  );
}

export default memo(HeadersTabInner);
