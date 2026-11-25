import { Download } from "lucide-react";
import type { MastDownloadItem } from "../../shared/types/mast";
import { appModeStore } from "../../hooks/useAppMode";
import { downloadCountsOf, downloadsPillText, type DownloadCounts } from "./logic/downloadView";
import { readyDownloads, useDownloads } from "./state/downloadsStore";
import { dismissReady, setDrawerOpen, showReady } from "./state/downloadsController";
import { DownloadReadyView } from "./DownloadReady";
import "./archive.css";

interface WorkspaceDownloadsViewProps {
  counts: DownloadCounts;
  ready: readonly MastDownloadItem[];
  onDownloads: () => void;
  onShow: (item: MastDownloadItem) => void;
  onDismiss: (item: MastDownloadItem) => void;
}

export function WorkspaceDownloadsView({ counts, ready, onDownloads, onShow, onDismiss }: WorkspaceDownloadsViewProps) {
  const transferring = counts.downloading + counts.queued;
  if (transferring === 0 && ready.length === 0) return null;
  return (
    <div className="ab-mast-footer" data-testid="workspace-downloads">
      {transferring > 0 ? (
        <button
          type="button"
          data-testid="mast-downloads-pill"
          data-busy="true"
          data-downloading={counts.downloading}
          data-queued={counts.queued}
          className="ab-mast-footer-btn"
          title="Show the MAST downloads in the Archive"
          onClick={onDownloads}
        >
          <Download size={10} aria-hidden="true" />
          {downloadsPillText(counts)}
        </button>
      ) : null}
      <DownloadReadyView items={ready} showLabel="Show" onShow={onShow} onDismiss={onDismiss} />
    </div>
  );
}

export default function WorkspaceDownloads() {
  const state = useDownloads();
  return (
    <WorkspaceDownloadsView
      counts={downloadCountsOf(state.items)}
      ready={readyDownloads(state)}
      onDownloads={() => {
        appModeStore.set("archive");
        setDrawerOpen(true);
      }}
      onShow={(item) => void showReady(item)}
      onDismiss={(item) => dismissReady(item)}
    />
  );
}
