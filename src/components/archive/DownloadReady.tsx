import { CheckCircle2, X } from "lucide-react";
import type { MastDownloadItem } from "../../shared/types/mast";
import { readyDownloads, useDownloads } from "./state/downloadsStore";
import { dismissReady, showReady } from "./state/downloadsController";

interface DownloadReadyViewProps {
  items: readonly MastDownloadItem[];
  showLabel: string;
  placement?: "float" | "inline";
  onShow: (item: MastDownloadItem) => void;
  onDismiss: (item: MastDownloadItem) => void;
}

function readyText(item: Pick<MastDownloadItem, "filename">): string {
  return `${item.filename} is downloaded and loaded in Files.`;
}

export function DownloadReadyView({ items, showLabel, placement = "inline", onShow, onDismiss }: DownloadReadyViewProps) {
  if (items.length === 0) return null;
  return (
    <div data-testid="mast-dl-ready" data-placement={placement} className="ab-mast-dl-ready" role="status">
      {items.map((item) => (
        <div key={item.id} data-id={item.id} className="ab-mast-dl-ready-row">
          <CheckCircle2 size={12} aria-hidden="true" />
          <span className="ab-mast-dl-ready-text ab-mast-mono" title={item.path}>
            {readyText(item)}
          </span>
          <button type="button" className="ab-mast-link-btn" onClick={() => onShow(item)}>
            {showLabel}
          </button>
          <button type="button" className="ab-mast-icon-btn" aria-label={`Dismiss ${item.filename}`} title="Dismiss" onClick={() => onDismiss(item)}>
            <X size={11} aria-hidden="true" />
          </button>
        </div>
      ))}
    </div>
  );
}

export default function DownloadReady({ showLabel }: { showLabel: string }) {
  const state = useDownloads();
  return <DownloadReadyView items={readyDownloads(state)} showLabel={showLabel} placement="float" onShow={(item) => void showReady(item)} onDismiss={(item) => dismissReady(item)} />;
}
