import { useEffect, type KeyboardEvent } from "react";
import { AlertTriangle, FolderOpen, X } from "lucide-react";
import type { MastDownloadItem, MastError } from "../../shared/types/mast";
import { ACTION_LABELS, discardable, downloadRoot, downloadStatusText, openableDownload, partialText, progressPercent } from "./logic/downloadView";
import { rateOf, useDownloads } from "./state/downloadsStore";
import { cancelDownload, discardDownload, openDownloaded, recoverDownload, revealDownload, setDrawerOpen, watchDownloads } from "./state/downloadsController";

interface DownloadQueueViewProps {
  items: readonly MastDownloadItem[];
  cancelError?: MastError | null;
  rates?: Readonly<Record<string, number | null>>;
  onCancel: (item: MastDownloadItem) => void;
  onAction: (item: MastDownloadItem) => void;
  onOpen: (item: MastDownloadItem) => void;
  onReveal: (item: MastDownloadItem) => void;
  onDiscard?: (item: MastDownloadItem) => void;
  onClose: () => void;
}

type RowProps = Omit<DownloadQueueViewProps, "items" | "cancelError" | "onClose" | "rates"> & { item: MastDownloadItem; rate: number | null };

function DownloadRow({ item, rate, onCancel, onAction, onOpen, onReveal, onDiscard }: RowProps) {
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
        {downloadStatusText(item, rate)}
      </span>
      {share !== null ? (
        <div role="progressbar" aria-label={`${item.filename} progress`} aria-valuemin={0} aria-valuemax={100} aria-valuenow={share} className="ab-mast-dl-progress">
          <span className="ab-mast-dl-progress-fill" style={{ width: `${share}%` }} />
        </div>
      ) : null}
      <span className="ab-mast-dl-actions">
        {active ? (
          <button
            type="button"
            className="ab-mast-link-btn"
            title={item.state === "downloading" ? "Stop now; Resume continues from the bytes already on disk" : undefined}
            onClick={() => onCancel(item)}
          >
            {item.state === "downloading" ? "Pause" : "Cancel"}
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
        {onDiscard && discardable(item) ? (
          <button type="button" className="ab-mast-link-btn" title="Delete the partial file from the disk" onClick={() => onDiscard(item)}>
            Discard
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

export function DownloadQueueView({ items, cancelError = null, rates = {}, onCancel, onAction, onOpen, onReveal, onDiscard, onClose }: DownloadQueueViewProps) {
  const root = downloadRoot(items);
  const newest = items.length ? items[items.length - 1] : null;
  const partial = partialText(items);
  const onKeyDown = (e: KeyboardEvent<HTMLElement>) => {
    if (e.key !== "Escape") return;
    e.preventDefault();
    e.stopPropagation();
    onClose();
  };
  return (
    <section data-testid="mast-downloads" data-mast-region="downloads" tabIndex={-1} className="ab-mast-dl-drawer" aria-label="MAST downloads" onKeyDown={onKeyDown}>
      <header className="ab-mast-dl-head">
        <span className="ab-mast-pgroup-title">Downloads</span>
        <button type="button" className="ab-mast-link-btn" aria-label="Close downloads" title="Close (Esc)" onClick={onClose}>
          <X size={12} aria-hidden="true" />
        </button>
      </header>
      {root && newest ? (
        <div data-testid="mast-dl-root" className="ab-mast-dl-root">
          <span className="ab-mast-dl-root-path ab-mast-mono" title={root}>
            {`Saved under ${root}`}
          </span>
          <button type="button" className="ab-mast-link-btn" onClick={() => onReveal(newest)}>
            <FolderOpen size={10} aria-hidden="true" />
            Open folder
          </button>
        </div>
      ) : null}
      {partial ? (
        <p data-testid="mast-dl-partial" className="ab-mast-detail-note">
          {partial}
        </p>
      ) : null}
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
            <DownloadRow key={item.id} item={item} rate={rates[item.id] ?? null} onCancel={onCancel} onAction={onAction} onOpen={onOpen} onReveal={onReveal} onDiscard={onDiscard} />
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
  const rates = Object.fromEntries(state.items.map((item) => [item.id, rateOf(state, item.id)]));
  return (
    <DownloadQueueView
      items={state.items}
      cancelError={state.cancelError}
      rates={rates}
      onCancel={(item) => void cancelDownload(item)}
      onDiscard={(item) => void discardDownload(item)}
      onAction={(item) => void recoverDownload(item)}
      onOpen={(item) => {
        openDownloaded(item);
      }}
      onReveal={(item) => void revealDownload(item).catch(() => undefined)}
      onClose={() => setDrawerOpen(false)}
    />
  );
}
