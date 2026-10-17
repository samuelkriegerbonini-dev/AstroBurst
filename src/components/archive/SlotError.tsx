import type { MouseEvent } from "react";
import { AlertCircle, AlertTriangle, Clock, CloudOff, HardDrive, ImageOff, Lock, XCircle, type LucideIcon } from "lucide-react";
import { slotTooltip, type SlotErrorView } from "./logic/errorCopy";

export type SlotSize = "tile" | "cell" | "stage";

interface SlotErrorProps {
  view: SlotErrorView;
  size: SlotSize;
  id?: string;
  noun?: string;
  text?: string;
  onRetry?: () => void;
  onCopy?: () => void;
}

const ICON_SIZE: Record<SlotSize, number> = { tile: 14, cell: 12, stage: 20 };

function iconOf(view: SlotErrorView): LucideIcon {
  switch (view.code) {
    case "mast_timeout":
    case "mast_rate_limited":
      return Clock;
    case "mast_network":
      return CloudOff;
    case "mast_decode":
    case "mast_not_image":
      return AlertCircle;
    case "mast_cache":
      return HardDrive;
    case "mast_unauthorized":
    case "mast_token_rejected":
      return Lock;
    case "mast_no_image":
      return view.badge === "EXCL" ? Lock : ImageOff;
    case "mast_too_large":
      return ImageOff;
    case "mast_not_deployed":
    case "mast_bad_input":
      return XCircle;
    default:
      return AlertTriangle;
  }
}

function stop(e: MouseEvent) {
  e.stopPropagation();
}

export default function SlotError({ view, size, id, noun = "Image", text, onRetry, onCopy }: SlotErrorProps) {
  const Icon = iconOf(view);
  const icon = <Icon size={ICON_SIZE[size]} aria-hidden="true" className="ab-mast-slot-icon" />;

  if (size === "cell") {
    return (
      <span className="ab-mast-slot-error" data-size="cell" data-tone={view.tone} data-code={view.code}>
        {icon}
        <span className="ab-mast-slot-badge">{view.badge}</span>
        <span className="sr-only">{`${noun} failed: ${view.summary}`}</span>
      </span>
    );
  }

  const retry =
    view.retry && onRetry ? (
      <button
        type="button"
        className={size === "tile" ? "ab-mast-link-btn ab-mast-slot-retry" : "ab-btn-secondary ab-mast-action"}
        tabIndex={size === "tile" ? -1 : undefined}
        onMouseDown={size === "tile" ? (e) => e.preventDefault() : undefined}
        onClick={(e) => {
          e.stopPropagation();
          onRetry();
        }}
        onDoubleClick={stop}
      >
        Retry
        <kbd className="ab-cmdp-kbd">R</kbd>
      </button>
    ) : null;

  if (size === "tile") {
    return (
      <div className="ab-mast-slot-error" data-size="tile" data-tone={view.tone} data-code={view.code} title={slotTooltip(view)}>
        {icon}
        <span className="ab-mast-slot-title">{view.title}</span>
        {view.meta ? <span className="ab-mast-slot-meta">{view.meta}</span> : null}
        {view.detail ? <span className="ab-mast-slot-detail">{view.detail}</span> : null}
        {retry}
        {id ? (
          <span id={id} className="sr-only">
            {`${noun} failed: ${view.summary}`}
          </span>
        ) : null}
      </div>
    );
  }

  return (
    <div className="ab-mast-slot-error" data-size="stage" data-tone={view.tone} data-code={view.code}>
      {icon}
      <p className="ab-mast-slot-title">{view.title}</p>
      {text ? <p className="ab-mast-slot-text">{text}</p> : null}
      {view.facts ? <p className="ab-mast-slot-meta">{view.facts}</p> : null}
      {view.detail ? <blockquote className="ab-mast-slot-detail">{view.detail}</blockquote> : null}
      <div className="ab-mast-slot-actions">
        {retry}
        {onCopy ? (
          <button type="button" className="ab-mast-link-btn" onClick={onCopy}>
            Copy details
          </button>
        ) : null}
      </div>
      <details className="ab-mast-details">
        <summary>Details</summary>
        <ul>
          {view.lines.map((line) => (
            <li key={line}>{line}</li>
          ))}
        </ul>
      </details>
    </div>
  );
}
