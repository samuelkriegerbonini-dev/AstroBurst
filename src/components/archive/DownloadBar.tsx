import { AlertTriangle, Download } from "lucide-react";
import type { MastError } from "../../shared/types/mast";
import { formatBytes } from "./logic/productView";

export type ConfirmKind = "plain" | "open";

interface DownloadBarViewProps {
  count: number;
  bytes: number;
  confirming: ConfirmKind | null;
  error: MastError | null;
  unavailable?: string | null;
  onDownload: (open: boolean) => void;
  onConfirm: () => void;
  onCancel: () => void;
}

function selectionText(count: number, bytes: number): string {
  if (count === 0) return "Select files to download";
  return `${count} ${count === 1 ? "file" : "files"} selected, ${formatBytes(bytes)}`;
}

function refusalText(error: MastError): string {
  const message = error.message.trim().replace(/[.\s]+$/, "");
  return message ? `Download refused: ${message}.` : "Download refused.";
}

export function DownloadBarView({ count, bytes, confirming, error, unavailable = null, onDownload, onConfirm, onCancel }: DownloadBarViewProps) {
  if (unavailable) {
    return (
      <div data-testid="mast-dl-bar" data-state="unavailable" className="ab-mast-dl-bar">
        <p className="ab-mast-detail-note">{unavailable}</p>
      </div>
    );
  }
  return (
    <div data-testid="mast-dl-bar" className="ab-mast-dl-bar">
      <div className="ab-mast-dl-bar-row">
        <span className="ab-mast-dl-selection">{selectionText(count, bytes)}</span>
        <button type="button" className="ab-btn-secondary ab-mast-action" disabled={count === 0} onClick={() => onDownload(false)}>
          <Download size={11} aria-hidden="true" />
          Download
        </button>
        <button type="button" className="ab-btn-secondary ab-mast-action" disabled={count === 0} onClick={() => onDownload(true)}>
          Download and open
        </button>
      </div>
      {confirming ? (
        <div data-testid="mast-dl-confirm" role="alertdialog" aria-label="Confirm a large download" className="ab-mast-dl-confirm">
          <span>{`Download ${formatBytes(bytes)} from MAST into the Data folder?`}</span>
          <button type="button" className="ab-btn-secondary ab-mast-action" onClick={onConfirm}>
            {confirming === "open" ? "Download and open" : "Download"}
          </button>
          <button type="button" className="ab-btn-secondary ab-mast-action" onClick={onCancel}>
            Cancel
          </button>
        </div>
      ) : null}
      {error && error.code !== "mast_cancelled" ? (
        <div className="ab-mast-banner" data-kind="server" role="alert">
          <AlertTriangle size={13} aria-hidden="true" />
          <span className="ab-mast-banner-text">{refusalText(error)}</span>
        </div>
      ) : null}
    </div>
  );
}
