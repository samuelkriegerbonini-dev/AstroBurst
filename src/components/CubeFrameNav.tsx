import { useState, useCallback, useRef, memo, useEffect, useId } from "react";
import { Loader2, Film, SkipBack, SkipForward, Play, Pause, Repeat } from "lucide-react";
import { getCubeFrame } from "../services/cube";
import {
  PLAYBACK_SPEEDS,
  clampFrame,
  cubeFrameOutputPaths,
  drainFrameRequest,
  formatFrameLabel,
  frameLoadPlan,
  isPlaybackSpeed,
  mergeFrameRequest,
  nextPlaybackFrame,
  playbackIntervalMs,
  playbackStartFrame,
  type FrameLoadRequest,
  type FramePublishGate,
  type PlaybackSpeed,
} from "../utils/cubeNavigation";

const SLIDER_DEBOUNCE_MS = 80;
const SLIDER_IDLE_COMMIT_MS = 400;
const FITS_COMMIT_DEBOUNCE_MS = 300;
const DEFAULT_SPEED: PlaybackSpeed = "normal";

type FrameKind = "transient" | "committed";

function publishAllowed(gate: FramePublishGate | undefined): boolean {
  return gate?.isCurrent() ?? true;
}

interface CubeFrameNavProps {
  filePath: string;
  totalFrames: number;
  frame: number;
  requestSeq: number;
  onFrameRequest: (idx: number) => void;
  frameLabel?: string | null;
  loop?: boolean;
  onLoopChange?: (loop: boolean) => void;
  onFrameChange?: (previewUrl: string, frameIndex: number, fitsPath?: string) => void;
  publishGate?: FramePublishGate;
}

function CubeFrameNavInner({
  filePath,
  totalFrames,
  frame,
  requestSeq,
  onFrameRequest,
  frameLabel = null,
  loop,
  onLoopChange,
  onFrameChange,
  publishGate,
}: CubeFrameNavProps) {
  const speedId = useId();
  const [dragValue, setDragValue] = useState<number | null>(null);
  const [loading, setLoading] = useState(false);
  const [playing, setPlaying] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [speed, setSpeed] = useState<PlaybackSpeed>(DEFAULT_SPEED);
  const [localLoop, setLocalLoop] = useState(false);
  const loopOn = loop ?? localLoop;

  const frameRef = useRef(frame);
  frameRef.current = frame;
  const loopRef = useRef(loopOn);
  loopRef.current = loopOn;
  const speedRef = useRef(speed);
  speedRef.current = speed;
  const onFrameChangeRef = useRef(onFrameChange);
  onFrameChangeRef.current = onFrameChange;
  const onFrameRequestRef = useRef(onFrameRequest);
  onFrameRequestRef.current = onFrameRequest;
  const publishGateRef = useRef(publishGate);
  publishGateRef.current = publishGate;

  const playingRef = useRef(false);
  const seqRef = useRef(0);
  const loadingRef = useRef(false);
  const pendingRef = useRef<FrameLoadRequest | null>(null);
  const pendingKindRef = useRef<FrameKind>("committed");
  const skipNextLoadRef = useRef(true);
  const sliderDirtyRef = useRef(false);
  const loadFrameRef = useRef<(idx: number, withFits: boolean) => void>(() => {});
  const sliderTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const sliderIdleTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const fitsTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const playTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pngCacheRef = useRef(new Map<number, string>());
  const fitsCacheRef = useRef(new Map<number, string>());

  const clearTimers = useCallback(() => {
    if (sliderTimerRef.current) clearTimeout(sliderTimerRef.current);
    if (sliderIdleTimerRef.current) clearTimeout(sliderIdleTimerRef.current);
    if (fitsTimerRef.current) clearTimeout(fitsTimerRef.current);
    if (playTimerRef.current) clearTimeout(playTimerRef.current);
    sliderTimerRef.current = null;
    sliderIdleTimerRef.current = null;
    fitsTimerRef.current = null;
    playTimerRef.current = null;
  }, []);

  useEffect(() => {
    const seq = seqRef;
    return () => {
      playingRef.current = false;
      seq.current++;
      clearTimers();
    };
  }, [clearTimers]);

  useEffect(() => {
    playingRef.current = false;
    setPlaying(false);
    setDragValue(null);
    setError(null);
    setLoading(false);
    seqRef.current++;
    loadingRef.current = false;
    pendingRef.current = null;
    pendingKindRef.current = "committed";
    sliderDirtyRef.current = false;
    skipNextLoadRef.current = true;
    clearTimers();
    pngCacheRef.current.clear();
    fitsCacheRef.current.clear();
  }, [filePath, clearTimers]);

  const loadFrame = useCallback(
    async (idx: number, withFits: boolean) => {
      if (idx < 0 || idx >= totalFrames) return;
      const png = pngCacheRef.current.get(idx);
      const fits = fitsCacheRef.current.get(idx);
      if (png && (!withFits || fits)) {
        if (publishAllowed(publishGateRef.current)) onFrameChangeRef.current?.(png, idx, withFits ? fits : undefined);
        return;
      }
      if (loadingRef.current) {
        pendingRef.current = mergeFrameRequest(pendingRef.current, { idx, withFits });
        return;
      }
      loadingRef.current = true;
      setLoading(true);
      setError(null);
      const seq = ++seqRef.current;
      try {
        const paths = cubeFrameOutputPaths(filePath, idx);
        const result = await getCubeFrame(filePath, idx, paths.png, withFits ? paths.fits : undefined);
        if (seqRef.current !== seq) return;
        pngCacheRef.current.set(idx, result.output_path);
        if (result.fits_path) fitsCacheRef.current.set(idx, result.fits_path);
        if (frameRef.current === idx && publishAllowed(publishGateRef.current)) {
          onFrameChangeRef.current?.(result.output_path, idx, result.fits_path ?? undefined);
        }
      } catch (e) {
        if (seqRef.current === seq) setError(e instanceof Error ? e.message : String(e));
        console.error("Frame load failed:", e);
      } finally {
        if (seqRef.current === seq) {
          loadingRef.current = false;
          setLoading(false);
          const pending = drainFrameRequest(pendingRef.current, frameRef.current);
          pendingRef.current = null;
          if (pending) loadFrameRef.current(pending.idx, pending.withFits);
        }
      }
    },
    [filePath, totalFrames],
  );
  loadFrameRef.current = loadFrame;

  const scheduleFits = useCallback((idx: number) => {
    if (fitsTimerRef.current) clearTimeout(fitsTimerRef.current);
    fitsTimerRef.current = setTimeout(() => {
      fitsTimerRef.current = null;
      if (frameRef.current === idx && publishAllowed(publishGateRef.current)) loadFrameRef.current(idx, true);
    }, FITS_COMMIT_DEBOUNCE_MS);
  }, []);

  useEffect(() => {
    const kind = pendingKindRef.current;
    pendingKindRef.current = "committed";
    if (skipNextLoadRef.current) {
      skipNextLoadRef.current = false;
      return;
    }
    const idx = frameRef.current;
    publishGateRef.current?.commit();
    if (fitsTimerRef.current) {
      clearTimeout(fitsTimerRef.current);
      fitsTimerRef.current = null;
    }
    const plan = frameLoadPlan(kind === "committed", fitsCacheRef.current.has(idx));
    loadFrameRef.current(idx, plan.withFits);
    if (plan.deferFits) scheduleFits(idx);
  }, [requestSeq, filePath, scheduleFits]);

  const requestFrame = useCallback(
    (idx: number, kind: FrameKind) => {
      const target = clampFrame(idx, totalFrames);
      if (target === frameRef.current) {
        if (kind === "committed") {
          publishGateRef.current?.commit();
          scheduleFits(target);
        }
        return;
      }
      pendingKindRef.current = kind;
      onFrameRequestRef.current(target);
    },
    [totalFrames, scheduleFits],
  );

  const commitSlider = useCallback(
    (idx: number) => {
      if (!sliderDirtyRef.current) return;
      sliderDirtyRef.current = false;
      if (sliderTimerRef.current) clearTimeout(sliderTimerRef.current);
      if (sliderIdleTimerRef.current) clearTimeout(sliderIdleTimerRef.current);
      sliderTimerRef.current = null;
      sliderIdleTimerRef.current = null;
      setDragValue(null);
      requestFrame(idx, "committed");
    },
    [requestFrame],
  );

  const handleSlider = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const idx = clampFrame(parseInt(e.target.value, 10), totalFrames);
      setDragValue(idx);
      sliderDirtyRef.current = true;
      if (sliderTimerRef.current) clearTimeout(sliderTimerRef.current);
      if (sliderIdleTimerRef.current) clearTimeout(sliderIdleTimerRef.current);
      sliderTimerRef.current = setTimeout(() => {
        sliderTimerRef.current = null;
        requestFrame(idx, "transient");
      }, SLIDER_DEBOUNCE_MS);
      sliderIdleTimerRef.current = setTimeout(() => {
        sliderIdleTimerRef.current = null;
        commitSlider(idx);
      }, SLIDER_IDLE_COMMIT_MS);
    },
    [totalFrames, requestFrame, commitSlider],
  );

  const handleSliderRelease = useCallback(
    (e: React.SyntheticEvent<HTMLInputElement>) => {
      commitSlider(clampFrame(parseInt(e.currentTarget.value, 10), totalFrames));
    },
    [totalFrames, commitSlider],
  );

  const stopPlayback = useCallback(() => {
    playingRef.current = false;
    setPlaying(false);
    if (playTimerRef.current) clearTimeout(playTimerRef.current);
    playTimerRef.current = null;
    requestFrame(frameRef.current, "committed");
  }, [requestFrame]);

  const handlePrev = useCallback(() => {
    requestFrame(frameRef.current - 1, "committed");
  }, [requestFrame]);

  const handleNext = useCallback(() => {
    requestFrame(frameRef.current + 1, "committed");
  }, [requestFrame]);

  const togglePlay = useCallback(() => {
    if (playingRef.current) {
      stopPlayback();
      return;
    }
    const first = playbackStartFrame(frameRef.current, totalFrames);
    if (first === null) return;
    playingRef.current = true;
    setPlaying(true);
    let cursor = frameRef.current;
    let firstStep = true;
    const tick = () => {
      playTimerRef.current = null;
      if (!playingRef.current) return;
      const next = firstStep ? first : nextPlaybackFrame(cursor, totalFrames, loopRef.current);
      firstStep = false;
      if (next === null) {
        stopPlayback();
        return;
      }
      cursor = next;
      requestFrame(next, "transient");
      playTimerRef.current = setTimeout(tick, playbackIntervalMs(speedRef.current));
    };
    tick();
  }, [totalFrames, stopPlayback, requestFrame]);

  const toggleLoop = useCallback(() => {
    const next = !loopRef.current;
    setLocalLoop(next);
    onLoopChange?.(next);
  }, [onLoopChange]);

  const handleSpeed = useCallback((e: React.ChangeEvent<HTMLSelectElement>) => {
    if (isPlaybackSpeed(e.target.value)) setSpeed(e.target.value);
  }, []);

  if (totalFrames <= 1) return null;

  const shown = dragValue ?? frame;
  const headerText = dragValue === null && frameLabel ? frameLabel : formatFrameLabel(shown, totalFrames, null, "");

  return (
    <div className="bg-zinc-950/50 rounded-lg border border-purple-500/20 p-3">
      <div className="flex items-center justify-between mb-2 gap-2">
        <span className="text-[10px] font-semibold text-purple-400 uppercase tracking-wider flex items-center gap-1.5 shrink-0">
          <Film size={11} />
          Frame Navigation
        </span>
        <span className="text-[10px] font-mono text-zinc-400 truncate" title={headerText}>
          {headerText}
          {loading && <Loader2 size={10} className="inline ml-1 animate-spin" />}
        </span>
      </div>

      <input
        type="range"
        min={0}
        max={totalFrames - 1}
        value={shown}
        onChange={handleSlider}
        onPointerUp={handleSliderRelease}
        onKeyUp={handleSliderRelease}
        aria-label="Cube frame"
        className="w-full accent-purple-500 mb-2"
      />

      <div className="flex items-center justify-center gap-2">
        <button
          onClick={handlePrev}
          disabled={shown === 0 || playing}
          title="Previous frame"
          aria-label="Previous frame"
          className="p-1.5 rounded-md transition-colors text-zinc-500 hover:text-zinc-200 hover:bg-zinc-800 disabled:opacity-30 disabled:cursor-not-allowed"
        >
          <SkipBack size={14} />
        </button>
        <button
          onClick={togglePlay}
          title={playing ? "Pause playback" : "Play all frames"}
          aria-label={playing ? "Pause playback" : "Play all frames"}
          className="p-1.5 rounded-md transition-colors hover:bg-zinc-800 disabled:opacity-30 disabled:cursor-not-allowed"
          style={{ color: playing ? "#c084fc" : "#71717a" }}
        >
          {playing ? <Pause size={14} /> : <Play size={14} />}
        </button>
        <button
          onClick={handleNext}
          disabled={shown === totalFrames - 1 || playing}
          title="Next frame"
          aria-label="Next frame"
          className="p-1.5 rounded-md transition-colors text-zinc-500 hover:text-zinc-200 hover:bg-zinc-800 disabled:opacity-30 disabled:cursor-not-allowed"
        >
          <SkipForward size={14} />
        </button>
        <button
          onClick={toggleLoop}
          aria-pressed={loopOn}
          title={loopOn ? "Loop playback: on (wraps to the first frame)" : "Loop playback: off (stops at the last frame)"}
          aria-label="Loop playback"
          className="p-1.5 rounded-md transition-colors hover:bg-zinc-800"
          style={{ color: loopOn ? "#c084fc" : "#71717a" }}
        >
          <Repeat size={14} />
        </button>
        <label htmlFor={speedId} className="sr-only">
          Playback speed
        </label>
        <select
          id={speedId}
          value={speed}
          onChange={handleSpeed}
          title="Playback speed"
          className="bg-zinc-900 border border-zinc-700/50 rounded px-1.5 py-0.5 text-[10px] text-zinc-200 focus:border-violet-500/50"
        >
          {PLAYBACK_SPEEDS.map((s) => (
            <option key={s.id} value={s.id}>
              {s.label} ({s.ms} ms)
            </option>
          ))}
        </select>
      </div>

      {error && (
        <p className="text-[9px] text-red-400/80 mt-1.5 text-center truncate" title={error}>
          Frame load failed: {error}
        </p>
      )}
    </div>
  );
}

export default memo(CubeFrameNavInner);
