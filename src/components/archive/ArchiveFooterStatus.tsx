import { KeyRound, LockOpen } from "lucide-react";
import { useArchiveState } from "./state/archiveStore";
import { useInflight } from "./state/inflight";
import { archiveController } from "./state/archiveController";
import { romanLockOf } from "./state/archiveReducer";
import { footerMastText, queuedThumbs } from "./logic/resultsView";
import "./archive.css";

interface FooterStatusViewProps {
  total: number;
  thumbs: number;
  queued: number;
  romanUnlocked: boolean;
  processing: { done: number; total: number } | null;
  onRoman: () => void;
}

export function FooterStatusView({ total, thumbs, queued, romanUnlocked, processing, onRoman }: FooterStatusViewProps) {
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
  return (
    <FooterStatusView
      total={inflight.total}
      thumbs={inflight.thumbs}
      queued={queuedThumbs(state.thumbs)}
      romanUnlocked={romanLockOf(state) === "unlocked"}
      processing={processing}
      onRoman={() => archiveController.openRomanAccess()}
    />
  );
}
