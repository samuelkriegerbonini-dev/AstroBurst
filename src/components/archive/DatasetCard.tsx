import { memo } from "react";
import { ImageOff, Lock } from "lucide-react";
import type { MastRow } from "../../shared/types/mast";
import type { ThumbStatus } from "./logic/thumbQueue";
import type { ThumbRef } from "./state/archiveReducer";
import { cardAccessibleName, dateText, exclusiveText, exposureText, filterRangeText, instrumentText, levelBadge } from "./logic/cardLabel";

interface DatasetCardProps {
  row: MastRow;
  missionLabel: string;
  status: ThumbStatus;
  thumb: ThumbRef | undefined;
  reason: string | undefined;
  selected: boolean;
  tabbable: boolean;
  onPick: (datasetId: string) => void;
  onFocusCard: (datasetId: string) => void;
  onOpen?: (datasetId: string) => void;
}

function DatasetCard({ row, missionLabel, status, thumb, reason, selected, tabbable, onPick, onFocusCard, onOpen }: DatasetCardProps) {
  const exclusive = exclusiveText(row);
  const level = levelBadge(row.levelClass);
  const instrument = instrumentText(row.instrument);
  const title = row.target?.trim() || row.datasetId;
  const meta = [dateText(row.dateObs), exposureText(row.expTimeS)].filter(Boolean).join("  ");
  return (
    <div
      role="gridcell"
      data-testid="mast-card"
      data-dataset={row.datasetId}
      data-thumb={status}
      data-mast-focus={tabbable ? "true" : undefined}
      aria-selected={selected}
      aria-label={cardAccessibleName(row, missionLabel)}
      tabIndex={tabbable ? 0 : -1}
      title={`${title}\n${row.datasetId}`}
      className="ab-mast-card"
      onClick={() => onPick(row.datasetId)}
      onDoubleClick={() => onOpen?.(row.datasetId)}
      onFocus={() => onFocusCard(row.datasetId)}
    >
      <div className="ab-mast-well">
        {status === "ready" && thumb ? (
          <img className="ab-mast-thumb" src={thumb.url} alt="" width={thumb.width} height={thumb.height} draggable={false} decoding="async" />
        ) : status === "failed" ? (
          <span className="ab-mast-thumb-note" title={`No preview (${reason ?? "unknown"})`}>
            <ImageOff size={18} aria-hidden="true" />
            No preview
          </span>
        ) : status === "paused" ? (
          <span className="ab-mast-thumb-note">paused</span>
        ) : (
          <span className="ab-mast-placeholder ab-mast-shimmer" aria-hidden="true" />
        )}
        {exclusive ? (
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
        <span className="ab-mast-card-meta">{filterRangeText(row.filters)}</span>
        <span className="ab-mast-card-meta">{meta}</span>
        {exclusive ? <span className="ab-mast-card-excl">{exclusive}</span> : null}
      </div>
    </div>
  );
}

export default memo(DatasetCard);
