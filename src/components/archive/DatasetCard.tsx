import { memo, useId } from "react";
import { Lock } from "lucide-react";
import type { MastRow } from "../../shared/types/mast";
import type { ThumbFailure, ThumbStatus } from "./logic/thumbQueue";
import type { ThumbRef } from "./state/archiveReducer";
import SlotError from "./SlotError";
import { slotError, type SlotErrorView } from "./logic/errorCopy";
import { cardAccessibleName, categoryText, dateText, exclusiveText, exposureText, filterRangeText, instrumentText, isPublic, levelBadge } from "./logic/cardLabel";

export const SLOW_LOADING_SEC = 3;

interface DatasetCardProps {
  row: MastRow;
  rowKey: string;
  missionLabel: string;
  status: ThumbStatus;
  thumb: ThumbRef | undefined;
  failure: ThumbFailure | undefined;
  paused: SlotErrorView | null;
  loadingSec: number | null;
  locked: boolean;
  selected: boolean;
  tabbable: boolean;
  onPick: (datasetId: string, rowKey: string) => void;
  onFocusCard: (datasetId: string, rowKey: string) => void;
  onOpen?: (datasetId: string) => void;
  onRetry?: (datasetId: string) => void;
  onThumbError?: (url: string) => void;
}

function DatasetCard({ row, rowKey, missionLabel, status, thumb, failure, paused, loadingSec, locked, selected, tabbable, onPick, onFocusCard, onOpen, onRetry, onThumbError }: DatasetCardProps) {
  const errorId = useId();
  const exclusive = exclusiveText(row);
  const level = levelBadge(row);
  const category = categoryText(row);
  const instrument = instrumentText(row.instrument);
  const title = row.target?.trim() || row.datasetId;
  const meta = [dateText(row.dateObs), exposureText(row.expTimeS)].filter(Boolean).join("  ");
  const failed = !locked && status === "failed" && failure ? slotError(failure.error, { surface: "thumb", rowPublic: isPublic(row), waitedMs: failure.waitedMs }) : null;
  const slot = failed ?? (!locked && status === "paused" ? paused : null);
  return (
    <div
      role="gridcell"
      data-testid="mast-card"
      data-dataset={row.datasetId}
      data-row-key={rowKey}
      data-thumb={locked ? "locked" : status}
      data-error-code={failed ? failed.code : undefined}
      data-error-status={failed ? failure?.error.context.status : undefined}
      data-error-waited-ms={failed && failure?.waitedMs !== null ? failure?.waitedMs : undefined}
      data-mast-focus={tabbable ? "true" : undefined}
      aria-selected={selected}
      aria-label={cardAccessibleName(row, missionLabel)}
      aria-describedby={slot ? errorId : undefined}
      tabIndex={tabbable ? 0 : -1}
      title={`${title}\n${row.datasetId}`}
      className="ab-mast-card"
      onClick={() => onPick(row.datasetId, rowKey)}
      onDoubleClick={() => onOpen?.(row.datasetId)}
      onFocus={() => onFocusCard(row.datasetId, rowKey)}
    >
      <div className="ab-mast-well">
        {locked ? (
          <span className="ab-mast-thumb-note ab-mast-thumb-locked" title="MAST serves no preview of exclusive-access data to anonymous users">
            <Lock size={18} aria-hidden="true" />
            EXCL.
          </span>
        ) : status === "ready" && thumb ? (
          <img
            className="ab-mast-thumb"
            src={thumb.url}
            alt=""
            width={thumb.width}
            height={thumb.height}
            draggable={false}
            decoding="async"
            onError={() => onThumbError?.(thumb.url)}
          />
        ) : slot ? (
          <SlotError view={slot} size="tile" id={errorId} noun="Thumbnail" onRetry={onRetry ? () => onRetry(row.datasetId) : undefined} />
        ) : status === "loading" ? (
          <span className="ab-mast-wait">
            <span className="ab-mast-placeholder ab-mast-shimmer" aria-hidden="true" />
            {loadingSec !== null && loadingSec >= SLOW_LOADING_SEC ? <span className="ab-mast-wait-text">{`Loading · ${loadingSec} s`}</span> : null}
          </span>
        ) : (
          <span className="ab-mast-wait">
            <span className="ab-mast-placeholder ab-mast-queued" aria-hidden="true" />
            <span className="ab-mast-wait-text">Queued</span>
          </span>
        )}
        {exclusive && !locked ? (
          <span className="ab-mast-lock-badge">
            <Lock size={9} aria-hidden="true" />
            EXCL.
          </span>
        ) : null}
        <span className="ab-mast-badges">
          {instrument ? <span className="ab-mast-badge">{instrument}</span> : null}
          {level ? <span className="ab-mast-badge">{level}</span> : null}
          {thumb?.imageCount ? (
            <span className="ab-mast-badge ab-mast-badge-count" title={`${thumb.imageCount} images`}>
              {thumb.imageCount}
            </span>
          ) : null}
        </span>
      </div>
      <div className="ab-mast-card-body">
        <span className="ab-mast-card-title">{title}</span>
        {category ? (
          <span className="ab-mast-card-cat" title={row.targetDescription ?? undefined}>
            {category}
          </span>
        ) : null}
        <span className="ab-mast-card-meta">{filterRangeText(row.filters)}</span>
        <span className="ab-mast-card-meta">{meta}</span>
        {exclusive ? <span className="ab-mast-card-excl">{exclusive}</span> : null}
      </div>
    </div>
  );
}

export default memo(DatasetCard);
