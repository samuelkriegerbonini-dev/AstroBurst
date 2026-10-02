import { useEffect, useMemo, useRef, useState } from "react";
import { Loader2, Maximize2, Minimize2 } from "lucide-react";
import ZoomPanView from "../ui/ZoomPanView";
import { channelOverlayPreview } from "../../services/compose";
import type { ChannelOverlayPreview } from "../../shared/types/compose";
import {
  alignDisplayedOnLoad,
  alignOverlayRequest,
  alignPreviewFrame,
  alignPreviewLegend,
  alignViewerState,
  MAX_OVERLAY_CHANNELS,
  type AlignedRun,
  type AlignOverlayShown,
  type AlignPreviewFrame,
} from "../../utils/wizard";

const BLINK_INTERVAL_MS = 500;

interface OverlayOutcome {
  loading: boolean;
  preview: ChannelOverlayPreview | null;
  binIds: readonly string[];
  error: string;
}

const EMPTY: OverlayOutcome = { loading: false, preview: null, binIds: [], error: "" };

function markPending(outcome: OverlayOutcome): OverlayOutcome {
  return { ...outcome, loading: true, error: "" };
}

function shownOutcome(outcome: OverlayOutcome): AlignOverlayShown | null {
  return outcome.preview
    ? { previewUrl: outcome.preview.previewUrl, frameUrls: outcome.preview.channelPreviewUrls, binIds: outcome.binIds }
    : null;
}

const SELECT_CLASS =
  "bg-zinc-900/80 border border-zinc-700/50 rounded text-[10px] text-zinc-300 px-1 py-0.5 cursor-pointer";

function segmentClass(active: boolean): string {
  return `px-1.5 py-0.5 text-[10px] transition-colors ${
    active ? "text-sky-300 bg-sky-500/15" : "text-zinc-500 hover:text-zinc-300"
  }`;
}

function errorText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

interface AlignPreviewProps {
  run: AlignedRun | null;
  labels: Readonly<Record<string, string>>;
  aligning?: boolean;
}

export default function AlignPreview({ run, labels, aligning = false }: AlignPreviewProps) {
  const [chosen, setChosen] = useState<string[]>([]);
  const [view, setView] = useState<"after" | "before">("after");
  const [blink, setBlink] = useState(false);
  const [blinkIndex, setBlinkIndex] = useState(1);
  const [showOther, setShowOther] = useState(false);
  const [tall, setTall] = useState(false);
  const [after, setAfter] = useState<OverlayOutcome>(EMPTY);
  const [before, setBefore] = useState<OverlayOutcome>(EMPTY);
  const [displayed, setDisplayed] = useState<AlignPreviewFrame | null>(null);
  const [brokenSrc, setBrokenSrc] = useState("");
  const seqRef = useRef(0);

  const request = useMemo(() => (run ? alignOverlayRequest(run, chosen) : null), [run, chosen]);

  useEffect(() => {
    const seq = ++seqRef.current;
    if (!request) {
      setAfter(EMPTY);
      setBefore(EMPTY);
      return;
    }
    setAfter(markPending);
    setBefore(markPending);
    const settle = (job: Promise<ChannelOverlayPreview>, set: (outcome: OverlayOutcome) => void) => {
      job.then(
        (preview) => {
          if (seq === seqRef.current) set({ loading: false, preview, binIds: request.binIds, error: "" });
        },
        (e: unknown) => {
          if (seq === seqRef.current) set({ loading: false, preview: null, binIds: [], error: errorText(e) });
        },
      );
    };
    settle(channelOverlayPreview(request.afterKeys, { withChannelFrames: true }), setAfter);
    settle(channelOverlayPreview(request.beforePaths), setBefore);
  }, [request]);

  const frames = after.preview?.channelPreviewUrls;

  const frame = alignPreviewFrame(
    view,
    { on: blink, showOther, index: blinkIndex },
    shownOutcome(after),
    shownOutcome(before),
    labels,
  );
  const blinking = frame.view === "blink";
  const current = frame.view === "before" ? before : after;

  const viewer = alignViewerState(frame, displayed, {
    hasRequest: request !== null,
    loading: current.loading,
    error: current.error,
    brokenSrc,
  });
  if (!viewer.src && displayed) setDisplayed(null);

  const handleImageLoad = (src: string) => {
    setDisplayed((shown) => alignDisplayedOnLoad(src, frame, shown));
  };
  const handleImageError = (src: string) => {
    if (src) setBrokenSrc(src);
  };

  useEffect(() => {
    if (!blinking) return;
    const timer = window.setInterval(() => setShowOther((v) => !v), BLINK_INTERVAL_MS);
    return () => window.clearInterval(timer);
  }, [blinking]);

  useEffect(() => {
    for (const url of frames ?? []) {
      const img = new Image();
      img.src = url;
    }
  }, [frames]);

  const requestLabels = request ? request.binIds.map((id) => labels[id] ?? id) : [];
  const shown = viewer.shown;
  const legend = shown
    ? alignPreviewLegend(shown.labels, shown.view, shown.blinkIndex)
    : request
      ? alignPreviewLegend(requestLabels, frame.view, frame.blinkIndex)
      : "";
  const loading = after.loading || before.loading;
  const blinkChoices = frame.canBlink ? after.binIds.slice(1) : [];

  const overlayChoices = run && request && run.binIds.length > MAX_OVERLAY_CHANNELS ? run.binIds.slice(1) : null;
  const pickOverlayChannel = (slot: 0 | 1, binId: string) => {
    if (!request) return;
    const next = request.binIds.slice(1);
    next[slot] = binId;
    setChosen(next);
  };

  const placeholder = !run
    ? aligning ? "Aligning…" : "Run Align to compare the channels here."
    : !request
      ? "Aligned channels are incomplete; run Align again."
      : "Rendering overlay…";

  return (
    <div className="flex flex-col flex-1 gap-1 min-w-0 select-none">
      <div className="flex flex-wrap items-center gap-1.5">
        <div role="group" aria-label="Overlay view" className="flex rounded-md overflow-hidden border border-zinc-700/40">
          <button
            type="button"
            aria-pressed={frame.view === "after"}
            onClick={() => { setView("after"); setBlink(false); }}
            className={segmentClass(frame.view === "after")}
            title="Aligned channels overlaid in colour"
          >
            After
          </button>
          <button
            type="button"
            aria-pressed={frame.view === "before"}
            onClick={() => { setView("before"); setBlink(false); }}
            className={segmentClass(frame.view === "before")}
            title="The inputs Align compared, overlaid in colour"
          >
            Before
          </button>
        </div>
        <button
          type="button"
          aria-pressed={blinking}
          disabled={!frame.canBlink}
          onClick={() => { setView("after"); setBlink((b) => !b); }}
          className={`rounded-md border border-zinc-700/40 disabled:opacity-40 disabled:cursor-not-allowed ${segmentClass(blinking)}`}
          title="Alternate the reference and one aligned channel every 0.5 s"
        >
          Blink
        </button>
        {blinkChoices.length > 1 && (
          <select
            aria-label="Channel blinked against the reference"
            value={frame.blinkIndex}
            onChange={(e) => setBlinkIndex(Number(e.target.value))}
            className={SELECT_CLASS}
          >
            {blinkChoices.map((id, i) => (
              <option key={id} value={i + 1}>{labels[id] ?? id}</option>
            ))}
          </select>
        )}
        {overlayChoices && request && (
          <span className="flex items-center gap-1 text-[10px] text-zinc-500">
            {(["G", "B"] as const).map((colour, slot) => (
              <label key={colour} className="flex items-center gap-0.5">
                {colour}
                <select
                  aria-label={colour === "G" ? "Channel shown in green" : "Channel shown in blue"}
                  value={request.binIds[slot + 1]}
                  onChange={(e) => pickOverlayChannel(slot as 0 | 1, e.target.value)}
                  className={SELECT_CLASS}
                >
                  {overlayChoices
                    .filter((id) => id !== request.binIds[2 - slot])
                    .map((id) => (
                      <option key={id} value={id}>{labels[id] ?? id}</option>
                    ))}
                </select>
              </label>
            ))}
          </span>
        )}
        <span className="ml-auto flex items-center gap-1.5">
          {loading && <Loader2 size={11} className="animate-spin text-zinc-500" aria-label="Rendering overlay" />}
          <button
            type="button"
            aria-pressed={tall}
            onClick={() => setTall((t) => !t)}
            title="Taller preview"
            aria-label="Taller preview"
            className="text-zinc-500 hover:text-zinc-300"
          >
            {tall ? <Minimize2 size={11} /> : <Maximize2 size={11} />}
          </button>
        </span>
      </div>

      <div
        className={`relative flex-1 rounded-md border border-zinc-800/60 bg-zinc-950 ${
          tall ? "min-h-[360px]" : "min-h-[140px]"
        }`}
      >
        {viewer.src ? (
          <>
            <div className="absolute inset-0">
              <ZoomPanView
                src={viewer.src}
                alt={shown?.label ?? ""}
                className="w-full h-full"
                preserveView
                wheelZoom="modifier"
                actualSizeButton
                onImageLoad={handleImageLoad}
                onImageError={handleImageError}
              />
            </div>
            {shown && (
              <span className="absolute top-1.5 left-1.5 z-10 text-[9px] font-mono text-zinc-300 bg-zinc-950/70 rounded px-1.5 py-0.5 pointer-events-none">
                {shown.label}
              </span>
            )}
            {viewer.pending && (
              <div className="absolute inset-0 z-10 flex items-center justify-center pointer-events-none">
                <span className="flex items-center gap-1.5 text-[10px] text-zinc-300 bg-zinc-950/75 rounded px-2 py-1">
                  <Loader2 size={12} className="animate-spin" />
                  Rendering {frame.label}…
                </span>
              </div>
            )}
            <span className="absolute bottom-1.5 right-11 z-10 text-[9px] text-zinc-400 bg-zinc-950/70 rounded px-1.5 py-0.5 pointer-events-none">
              Double-click or Ctrl+scroll to zoom
            </span>
          </>
        ) : (
          <div className="absolute inset-0 flex items-center justify-center px-3 text-center text-[10px]">
            {viewer.error
              ? <span className="text-red-400/90">Preview failed: {viewer.error}</span>
              : <span className="text-zinc-600">{placeholder}</span>}
          </div>
        )}
      </div>

      {legend && <div className="text-[9px] text-zinc-500 leading-snug">{legend}</div>}
    </div>
  );
}
