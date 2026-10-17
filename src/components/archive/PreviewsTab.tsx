import { useLayoutEffect, useRef, type KeyboardEvent } from "react";
import { ImageOff } from "lucide-react";
import type { MastImageEntry, MastMission, MastRow } from "../../shared/types/mast";
import { detailEntry, imageSetKey, type ArchiveState, type PreviewState } from "./state/archiveReducer";
import type { ArchiveController } from "./state/archiveController";
import { useNow } from "./state/useNow";
import {
  EXCLUSIVE_PREVIEWS_TEXT,
  cardThumbFor,
  entriesInGroup,
  entryFilename,
  entryTitle,
  groupsOf,
  listFailureText,
  listingText,
  previewAlt,
  previewFailureText,
  previewInfoLines,
  previewKey,
  previewStatusText,
  previewsAllowed,
  progressFraction,
  progressText,
  stripKey,
  stripPosition,
  stripThumbUrl,
  waitedOf,
} from "./logic/detailView";
import { previewsKeyCommand } from "./logic/detailKeys";
import { isTextEntry } from "./logic/archiveKeys";
import { isPublic } from "./logic/cardLabel";
import { thumbStatus } from "./logic/thumbQueue";
import { pausedSlot, slotCellTitle, slotError, type SlotErrorView } from "./logic/errorCopy";
import { getFilterColor } from "../../utils/filterColors";
import SlotError from "./SlotError";

interface PreviewsTabProps {
  state: ArchiveState;
  ctl: ArchiveController;
  row: MastRow;
  mission: MastMission;
}

const STRIP_SELECTOR = '[data-testid="mast-strip-item"]';
const NO_THUMBNAIL_TEXT = "MAST lists no thumbnail for this image";

export function SlotStage({ ctl, view, text, onRetry }: { ctl: ArchiveController; view: SlotErrorView; text: string; onRetry: () => void }) {
  return <SlotError view={view} size="stage" text={text} onRetry={onRetry} onCopy={() => void ctl.copy(view.lines.join("\n"), "the error details")} />;
}

export function ProgressBar({ fraction }: { fraction: number | null }) {
  if (fraction === null) {
    return <div role="progressbar" aria-label="Download progress" className="ab-mast-bar" data-indeterminate="true" />;
  }
  const pct = Math.round(fraction * 100);
  return (
    <div role="progressbar" aria-label="Download progress" aria-valuemin={0} aria-valuemax={100} aria-valuenow={pct} className="ab-mast-bar">
      <span className="ab-mast-bar-fill" style={{ width: `${pct}%` }} />
    </div>
  );
}

function RetryButton({ onRetry, label = "Retry" }: { onRetry: () => void; label?: string }) {
  return (
    <button type="button" className="ab-btn-secondary ab-mast-action" onClick={onRetry}>
      {label}
      <kbd className="ab-cmdp-kbd">R</kbd>
    </button>
  );
}

function Stage({
  ctl,
  row,
  entry,
  preview,
  backdrop,
  now,
}: {
  ctl: ArchiveController;
  row: MastRow;
  entry: MastImageEntry;
  preview: PreviewState | undefined;
  backdrop: string | null;
  now: number;
}) {
  const filename = entryFilename(entry);
  if (!entry.previewFilename) {
    return (
      <div data-testid="mast-preview-img" data-filename={filename} data-stage="failed" className="ab-mast-stage-msg">
        <ImageOff size={18} aria-hidden="true" />
        <p>MAST lists no preview for this image.</p>
      </div>
    );
  }
  if (preview?.stage === "ready" && preview.ref && preview.url) {
    const url = preview.url;
    return (
      <img
        data-testid="mast-preview-img"
        data-filename={filename}
        data-stage="ready"
        className="ab-mast-preview"
        src={url}
        alt={previewAlt(entry, preview.ref)}
        width={preview.ref.width}
        height={preview.ref.height}
        draggable={false}
        decoding="async"
        onDoubleClick={() => ctl.openLarge(row.datasetId, entry.key)}
        onError={() => ctl.imageMissing(url)}
      />
    );
  }
  if (preview?.stage === "failed" && preview.error) {
    const view = slotError(preview.error, { surface: "preview", rowPublic: isPublic(row), waitedMs: waitedOf(preview) });
    return (
      <div
        data-testid="mast-preview-img"
        data-filename={filename}
        data-stage="failed"
        data-error-code={preview.error.code}
        data-error-status={preview.error.context.status}
        className="ab-mast-stage-msg"
        role="alert"
      >
        {view ? (
          <SlotStage ctl={ctl} view={view} text={previewFailureText(preview.error, isPublic(row))} onRetry={() => ctl.retryDetail()} />
        ) : (
          <>
            <ImageOff size={18} aria-hidden="true" />
            <p>{previewFailureText(preview.error, isPublic(row))}</p>
            <RetryButton onRetry={() => ctl.retryDetail()} />
          </>
        )}
      </div>
    );
  }
  const phase = preview ? preview.stage : "pending";
  return (
    <div data-testid="mast-preview-img" data-filename={filename} data-stage={phase === "decode" ? "decode" : "download"} className="ab-mast-stage-wait" role="status" aria-busy="true">
      {backdrop ? <img className="ab-mast-stage-blur" src={backdrop} alt="" aria-hidden="true" draggable={false} /> : null}
      <div className="ab-mast-stage-panel">
        <p className="ab-mast-stage-text">{previewStatusText(phase, entry, "pane")}</p>
        {preview && phase === "download" ? <p className="ab-mast-mono ab-mast-stage-sub">{progressText(preview, now)}</p> : null}
        {preview ? <ProgressBar fraction={phase === "decode" ? 1 : progressFraction(preview)} /> : null}
        {preview ? (
          <button type="button" className="ab-btn-secondary ab-mast-action" onClick={() => ctl.cancelPreview("pane")}>
            Cancel
          </button>
        ) : null}
      </div>
    </div>
  );
}

export default function PreviewsTab({ state, ctl, row, mission }: PreviewsTabProps) {
  const boxRef = useRef<HTMLDivElement>(null);
  const followFocus = useRef(false);
  const key = imageSetKey(mission.id, row.datasetId);
  const set = state.imageSets[key];
  const list = state.lists[key];
  const entry = set ? detailEntry(state) ?? set.entries[0] ?? null : null;
  const paneKey = entry?.previewFilename ? previewKey(mission.id, row.datasetId, entry.previewFilename, "pane") : null;
  const preview = paneKey ? state.previews[paneKey] : undefined;
  const busy = (!set && !!list && !list.error) || preview?.stage === "download" || preview?.stage === "decode";
  const now = useNow(busy ? 250 : null);

  useLayoutEffect(() => {
    if (!followFocus.current) return;
    followFocus.current = false;
    const item = boxRef.current?.querySelector<HTMLElement>(`${STRIP_SELECTOR}[aria-current="true"]`);
    if (!item) return;
    item.focus({ preventScroll: true });
    item.scrollIntoView({ block: "nearest", inline: "nearest" });
  });

  if (!previewsAllowed(row, mission)) {
    return <p className="ab-mast-detail-note">{EXCLUSIVE_PREVIEWS_TEXT}</p>;
  }
  if (!set) {
    if (list?.error) {
      const view = slotError(list.error, { surface: "list", waitedMs: waitedOf(list) });
      return (
        <div className="ab-mast-detail-error" role="alert" data-error-code={list.error.code} data-error-status={list.error.context.status}>
          {view ? (
            <SlotStage ctl={ctl} view={view} text={listFailureText(list.error)} onRetry={() => ctl.retryDetail()} />
          ) : (
            <>
              <p>{listFailureText(list.error)}</p>
              <RetryButton onRetry={() => ctl.retryDetail()} />
            </>
          )}
        </div>
      );
    }
    return (
      <p className="ab-mast-detail-note" role="status">
        {listingText(row.datasetId, list ? Math.max(0, now - list.startedAt) : 0)}
      </p>
    );
  }
  if (!entry) return <p className="ab-mast-detail-note">MAST lists no preview images for this dataset.</p>;

  const groups = groupsOf(set);
  const strip = entriesInGroup(set, entry.group);
  const pos = stripPosition(set, entry);
  const card = cardThumbFor(set, state.thumbRefs[row.datasetId]);
  const backdrop = stripThumbUrl(row.datasetId, entry, state.thumbRefs, card);
  const paused = state.thumbs.pausedUntil !== null ? pausedSlot(state.thumbs, now) : null;

  const failedStripKey = (target: HTMLElement | null): string | null => {
    const item = target?.closest<HTMLElement>(STRIP_SELECTOR);
    if (!item || item.dataset.thumb !== "failed") return null;
    const picked = set.entries.find((x) => entryFilename(x) === item.dataset.filename);
    return picked?.thumbnailFilename ? stripKey(row.datasetId, picked.thumbnailFilename) : null;
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const target = e.target instanceof HTMLElement ? e.target : null;
    const plain = !e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey;
    const retryKey = plain && e.key.toLowerCase() === "r" ? failedStripKey(target) : null;
    if (retryKey) {
      e.preventDefault();
      ctl.retryThumbs([retryKey]);
      return;
    }
    const cmd = previewsKeyCommand(e, { inInput: isTextEntry(target) });
    if (!cmd) return;
    const onStrip = !!target?.closest(STRIP_SELECTOR);
    if (cmd === "open-large" && target?.closest("button") && !onStrip) return;
    e.preventDefault();
    if (cmd === "prev-image" || cmd === "next-image") {
      followFocus.current = onStrip;
      ctl.stepImage(cmd === "next-image" ? 1 : -1);
    } else if (cmd === "next-group") {
      followFocus.current = onStrip;
      ctl.cycleGroup();
    } else {
      const picked = onStrip ? set.entries.find((x) => entryFilename(x) === target?.closest<HTMLElement>(STRIP_SELECTOR)?.dataset.filename) : undefined;
      ctl.openLarge(row.datasetId, (picked ?? entry).key);
    }
  };

  return (
    <div ref={boxRef} className="ab-mast-previews" onKeyDown={onKeyDown}>
      {groups.length > 1 ? (
        <div className="ab-mast-groups" role="group" aria-label="Product group">
          <span className="ab-mast-filter-label">{mission.id === "hst" ? "Group" : "Product"}</span>
          <div className="ab-mast-segment">
            {groups.map((g) => (
              <button
                key={g.key}
                type="button"
                className="ab-mast-seg"
                data-active={g.key === entry.group ? "true" : "false"}
                aria-pressed={g.key === entry.group}
                title="Next group (G)"
                onClick={() => ctl.selectGroup(g.key)}
              >
                {`${g.label} ${g.count}`}
              </button>
            ))}
          </div>
        </div>
      ) : null}
      <div className="ab-mast-strip-row">
        <div className="ab-mast-strip" role="group" aria-label="Images in this group">
          {strip.map((e) => {
            const current = e.key === entry.key;
            const url = stripThumbUrl(row.datasetId, e, state.thumbRefs, card);
            const thumbKey = e.thumbnailFilename ? stripKey(row.datasetId, e.thumbnailFilename) : null;
            const status = thumbKey ? (url ? "ready" : thumbStatus(state.thumbs, thumbKey, now)) : "none";
            const failure = thumbKey && status === "failed" ? state.thumbs.failures[thumbKey] : undefined;
            const failed = failure ? slotError(failure.error, { surface: "thumb", rowPublic: isPublic(row), waitedMs: failure.waitedMs }) : null;
            const slot = failed ?? (status === "paused" ? paused : null);
            const title = !thumbKey ? `${entryTitle(e)}\n${NO_THUMBNAIL_TEXT}` : slot ? `${entryTitle(e)}\n${slotCellTitle(slot)}` : entryTitle(e);
            return (
              <button
                key={e.key}
                type="button"
                data-testid="mast-strip-item"
                data-filename={entryFilename(e)}
                data-thumb={status}
                data-error-code={failed ? failed.code : undefined}
                data-error-status={failure?.error.context.status}
                aria-current={current ? "true" : undefined}
                tabIndex={current ? 0 : -1}
                className="ab-mast-strip-item"
                title={title}
                onClick={() => ctl.selectImage(e.key)}
                onDoubleClick={() => ctl.openLarge(row.datasetId, e.key)}
              >
                <span className="ab-mast-strip-thumb">
                  {url ? (
                    <img src={url} alt="" draggable={false} decoding="async" onError={() => ctl.imageMissing(url)} />
                  ) : !thumbKey ? (
                    <span className="ab-mast-strip-none">
                      <ImageOff size={12} aria-hidden="true" />
                      <span className="sr-only">{NO_THUMBNAIL_TEXT}</span>
                    </span>
                  ) : slot ? (
                    <SlotError view={slot} size="cell" noun="Thumbnail" />
                  ) : (
                    <span className={status === "loading" ? "ab-mast-placeholder ab-mast-shimmer" : "ab-mast-placeholder ab-mast-queued"} aria-hidden="true" />
                  )}
                </span>
                <span className="ab-mast-strip-label">
                  {e.filter ? <span className="ab-mast-dot" style={{ background: getFilterColor(e.filter) }} aria-hidden="true" /> : null}
                  <span className="ab-mast-strip-text">{e.label}</span>
                </span>
                {e.colour ? <span className="ab-mast-badge">colour</span> : null}
              </button>
            );
          })}
        </div>
        <span className="ab-mast-strip-pos ab-mast-mono">{`${pos.index}/${pos.count}`}</span>
      </div>
      <div className="ab-mast-stage">
        <Stage ctl={ctl} row={row} entry={entry} preview={preview} backdrop={backdrop} now={now} />
      </div>
      <div className="ab-mast-preview-info">
        {preview?.stage === "ready" && preview.ref ? previewInfoLines(entry, preview.ref).map((line) => <p key={line}>{line}</p>) : <p>{entryTitle(entry)}</p>}
      </div>
      <div className="ab-mast-actions">
        <button type="button" className="ab-btn-secondary ab-mast-action" onClick={() => ctl.openLarge(row.datasetId, entry.key)}>
          Large view
          <kbd className="ab-cmdp-kbd">Space</kbd>
        </button>
        <button type="button" className="ab-btn-secondary ab-mast-action" onClick={() => void ctl.copy(entry.mastUri, "the MAST URI")}>
          Copy MAST URI
        </button>
      </div>
    </div>
  );
}
