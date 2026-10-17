import { Download, KeyRound, LockOpen } from "lucide-react";
import { useArchiveState } from "./state/archiveStore";
import { useInflight } from "./state/inflight";
import { archiveController } from "./state/archiveController";
import { romanLockOf } from "./state/archiveReducer";
import { footerMastText, queuedThumbs } from "./logic/resultsView";
import { downloadCountsOf, downloadsPillText, type DownloadCounts } from "./logic/downloadView";
import { useDownloads } from "./state/downloadsStore";
import { setDrawerOpen } from "./state/downloadsController";
import "./archive.css";

interface FooterStatusViewProps {
  total: number;
  thumbs: number;
  queued: number;
  romanUnlocked: boolean;
  processing: { done: number; total: number } | null;
  onRoman: () => void;
  downloads?: DownloadCounts;
  drawerOpen?: boolean;
  onDownloads?: () => void;
}

export function FooterStatusView({ total, thumbs, queued, romanUnlocked, processing, onRoman, downloads, drawerOpen = false, onDownloads }: FooterStatusViewProps) {
  const transferring = downloads ? downloads.downloading + downloads.queued : 0;
  return (
    <div className="ab-mast-footer">
      <span data-testid="mast-inflight" data-inflight={total} data-inflight-thumbs={thumbs} data-busy={total > 0 ? "true" : "false"} className="ab-mast-footer-pill">
        {footerMastText(total, thumbs, queued)}
      </span>
      <span className="ab-mast-footer-sep" aria-hidden="true">
        ·
      </span>
      <button
        type="button"
        data-testid="mast-roman-status"
        data-state={romanUnlocked ? "unlocked" : "locked"}
        className="ab-mast-footer-btn"
        title={romanUnlocked ? "Roman access confirmed" : "Roman needs an authorized MAST token"}
        onClick={onRoman}
      >
        {romanUnlocked ? <LockOpen size={10} aria-hidden="true" /> : <KeyRound size={10} aria-hidden="true" />}
        {romanUnlocked ? "Roman unlocked" : "Roman locked"}
      </button>
      {downloads && onDownloads ? (
        <>
          <span className="ab-mast-footer-sep" aria-hidden="true">
            ·
          </span>
          <button
            type="button"
            data-testid="mast-downloads-pill"
            data-busy={transferring > 0 ? "true" : "false"}
            data-downloading={downloads.downloading}
            data-queued={downloads.queued}
            aria-expanded={drawerOpen}
            className="ab-mast-footer-btn"
            title="Show the MAST downloads"
            onClick={onDownloads}
          >
            <Download size={10} aria-hidden="true" />
            {downloadsPillText(downloads)}
          </button>
        </>
      ) : null}
      {processing ? (
        <span data-testid="mast-processing" className="ab-mast-footer-pill" data-busy="true">
          {`Processing ${processing.done}/${processing.total}`}
        </span>
      ) : null}
    </div>
  );
}

export default function ArchiveFooterStatus({ processing }: { processing: { done: number; total: number } | null }) {
  const state = useArchiveState();
  const inflight = useInflight();
  const downloads = useDownloads();
  return (
    <FooterStatusView
      total={inflight.total}
      thumbs={inflight.thumbs}
      queued={queuedThumbs(state.thumbs)}
      romanUnlocked={romanLockOf(state) === "unlocked"}
      processing={processing}
      onRoman={() => archiveController.openRomanAccess()}
      downloads={downloadCountsOf(downloads.items)}
      drawerOpen={downloads.drawerOpen}
      onDownloads={() => setDrawerOpen(!downloads.drawerOpen)}
    />
  );
}
