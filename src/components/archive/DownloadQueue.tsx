import { useEffect } from "react";
import { AlertTriangle, FolderOpen, X } from "lucide-react";
import type { MastDownloadItem, MastError } from "../../shared/types/mast";
import { ACTION_LABELS, downloadStatusText, openableDownload, progressPercent } from "./logic/downloadView";
import { useDownloads } from "./state/downloadsStore";
import { cancelDownload, openDownloaded, recoverDownload, revealDownload, setDrawerOpen, watchDownloads } from "./state/downloadsController";

interface DownloadQueueViewProps {
  items: readonly MastDownloadItem[];
  cancelError?: MastError | null;
  onCancel: (item: MastDownloadItem) => void;
  onAction: (item: MastDownloadItem) => void;
  onOpen: (item: MastDownloadItem) => void;
  onReveal: (item: MastDownloadItem) => void;
  onClose: () => void;
}

type RowProps = Omit<DownloadQueueViewProps, "items" | "cancelError" | "onClose"> & { item: MastDownloadItem };

function DownloadRow({ item, onCancel, onAction, onOpen, onReveal }: RowProps) {
  const share = progressPercent(item);
  const active = item.state === "queued" || item.state === "downloading";
  const done = item.state === "done";
  const unverified = done && item.verified === false;
  return (
    <li
      data-testid="mast-dl-item"
      data-id={item.id}
      data-state={item.state}
      data-action={item.action ?? undefined}
      data-code={item.error?.code}
      data-verified={item.verified === null ? undefined : String(item.verified)}
      className="ab-mast-dl-item"
    >
      <span className="ab-mast-dl-name ab-mast-mono" title={item.path}>
        {item.filename}
      </span>
      <span className="ab-mast-dl-status" data-unverified={unverified ? "true" : undefined}>
        {downloadStatusText(item)}
      </span>
      {share !== null ? (
        <div role="progressbar" aria-label={`${item.filename} progress`} aria-valuemin={0} aria-valuemax={100} aria-valuenow={share} className="ab-mast-dl-progress">
          <span className="ab-mast-dl-progress-fill" style={{ width: `${share}%` }} />
        </div>
      ) : null}
      <span className="ab-mast-dl-actions">
        {active ? (
          <button type="button" className="ab-mast-link-btn" onClick={() => onCancel(item)}>
            Cancel
          </button>
        ) : null}
        {!active && item.action ? (
          <button type="button" className="ab-mast-link-btn" onClick={() => onAction(item)}>
            {ACTION_LABELS[item.action]}
          </button>
        ) : null}
        {done && openableDownload(item) ? (
          <button type="button" className="ab-mast-link-btn" onClick={() => onOpen(item)}>
            Open
          </button>
        ) : null}
        {done ? (
          <button type="button" className="ab-mast-link-btn" onClick={() => onReveal(item)}>
            <FolderOpen size={10} aria-hidden="true" />
            Show in folder
          </button>
        ) : null}
      </span>
    </li>
  );
}

function cancelFailureText(error: MastError): string {
  const message = error.message.trim().replace(/[.\s]+$/, "");
  return message ? `Cancel failed: ${message}.` : "Cancel failed.";
}

export function DownloadQueueView({ items, cancelError = null, onCancel, onAction, onOpen, onReveal, onClose }: DownloadQueueViewProps) {
  return (
    <section data-testid="mast-downloads" className="ab-mast-dl-drawer" aria-label="MAST downloads">
      <header className="ab-mast-dl-head">
        <span className="ab-mast-pgroup-title">Downloads</span>
        <button type="button" className="ab-mast-link-btn" aria-label="Close downloads" onClick={onClose}>
          <X size={12} aria-hidden="true" />
        </button>
      </header>
      {cancelError ? (
        <div data-testid="mast-dl-cancel-error" className="ab-mast-banner" data-kind="server" role="alert">
          <AlertTriangle size={13} aria-hidden="true" />
          <span className="ab-mast-banner-text">{cancelFailureText(cancelError)}</span>
        </div>
      ) : null}
      {items.length === 0 ? (
        <p className="ab-mast-detail-note">Nothing downloaded yet. Select files in the Products tab.</p>
      ) : (
        <ul className="ab-mast-dl-list">
          {[...items].reverse().map((item) => (
            <DownloadRow key={item.id} item={item} onCancel={onCancel} onAction={onAction} onOpen={onOpen} onReveal={onReveal} />
          ))}
        </ul>
      )}
    </section>
  );
}

export default function DownloadQueue() {
  const state = useDownloads();
  useEffect(() => {
    void watchDownloads();
  }, []);
  if (!state.drawerOpen) return null;
  return (
    <DownloadQueueView
      items={state.items}
      cancelError={state.cancelError}
      onCancel={(item) => void cancelDownload(item)}
      onAction={(item) => void recoverDownload(item)}
      onOpen={(item) => {
        openDownloaded(item);
      }}
      onReveal={(item) => void revealDownload(item).catch(() => undefined)}
      onClose={() => setDrawerOpen(false)}
    />
  );
}
