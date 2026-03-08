import { useState, useCallback, lazy, Suspense, memo } from "react";
import { Loader2 } from "lucide-react";
import { useBackend } from "../../hooks/useBackend";
import { usePreviewContext } from "../../context/PreviewContext";
import type { HeaderData } from "../../utils/types";

const HeaderExplorerPanel = lazy(() => import("./HeaderExplorerPanel"));
const HeaderTable = lazy(() => import("./HeaderTable"));
const HduSelectorPanel = lazy(() => import("./HduSelectorPanel"));

function HeadersTabInner() {
  const { file, setRgbChannels } = usePreviewContext();
  const { getFullHeader } = useBackend();

  const [headerData, setHeaderData] = useState<HeaderData | null>(null);
  const [headerLoading, setHeaderLoading] = useState(false);

  const handleLoadHeader = useCallback(
    async (path: string) => {
      setHeaderLoading(true);
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
    [getFullHeader],
  );

  const handleAssignChannel = useCallback(
    (channel: string, path: string) => {
      setRgbChannels((prev: any) => ({ ...prev, [channel.toLowerCase()]: path }));
    },
    [setRgbChannels],
  );

  return (
    <Suspense
      fallback={
        <div className="flex items-center justify-center py-12">
          <Loader2 size={20} className="animate-spin text-zinc-500" />
        </div>
      }
    >
      <div className="flex flex-col gap-4">
        {file?.path && (
          <HduSelectorPanel filePath={file.path} />
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

        {file?.result?.header && (
          <div className="bg-zinc-950/50 rounded-lg p-4 border border-zinc-800/50">
            <h4 className="text-xs font-semibold text-zinc-400 uppercase tracking-wider mb-3">
              FITS Header (Summary)
            </h4>
            <HeaderTable header={file.result.header} />
          </div>
        )}
      </div>
    </Suspense>
  );
}

export default memo(HeadersTabInner);
