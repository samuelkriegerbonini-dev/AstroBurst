import { useState, useCallback, useRef, memo, useEffect } from "react";
import { Loader2, Film, SkipBack, SkipForward, Play, Pause } from "lucide-react";
import { getCubeFrame } from "../services/cube.service";

interface CubeFrameNavProps {
  filePath: string;
  totalFrames: number;
  onFrameChange?: (previewUrl: string, frameIndex: number) => void;
}

function CubeFrameNavInner({ filePath, totalFrames, onFrameChange }: CubeFrameNavProps) {
  const [currentFrame, setCurrentFrame] = useState(0);
  const [loading, setLoading] = useState(false);
  const [playing, setPlaying] = useState(false);
  const playingRef = useRef(false);
  const abortRef = useRef(0);
  const loadingRef = useRef(false);
  const sliderTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    return () => {
      playingRef.current = false;
      if (sliderTimerRef.current) clearTimeout(sliderTimerRef.current);
    };
  }, []);

  const loadFrame = useCallback(
    async (idx: number) => {
      if (idx < 0 || idx >= totalFrames) return;
      if (loadingRef.current) return;
      loadingRef.current = true;
      setCurrentFrame(idx);
      setLoading(true);
      const seq = ++abortRef.current;
      try {
        const outputPath = `./output/cube_frame_${idx}.png`;
        const result = await getCubeFrame(filePath, idx, outputPath);
        if (abortRef.current !== seq) return;
        onFrameChange?.(result.output_path, idx);
      } catch (e) {
        console.error("Frame load failed:", e);
      } finally {
        if (abortRef.current === seq) setLoading(false);
        loadingRef.current = false;
      }
    },
    [filePath, totalFrames, getCubeFrame, onFrameChange],
  );

  const handleSlider = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const idx = parseInt(e.target.value);
      setCurrentFrame(idx);
      if (sliderTimerRef.current) clearTimeout(sliderTimerRef.current);
      sliderTimerRef.current = setTimeout(() => {
        sliderTimerRef.current = null;
        loadFrame(idx);
      }, 80);
    },
    [loadFrame],
  );

  const handlePrev = useCallback(() => {
    loadFrame(Math.max(0, currentFrame - 1));
  }, [currentFrame, loadFrame]);

  const handleNext = useCallback(() => {
    loadFrame(Math.min(totalFrames - 1, currentFrame + 1));
  }, [currentFrame, totalFrames, loadFrame]);

  const togglePlay = useCallback(() => {
    if (playing) {
      playingRef.current = false;
      setPlaying(false);
      return;
    }

    playingRef.current = true;
    setPlaying(true);

    let frame = currentFrame;

    const playNext = async () => {
      if (!playingRef.current) return;
      frame = (frame + 1) % totalFrames;

      setCurrentFrame(frame);
      setLoading(true);
      const seq = ++abortRef.current;
      try {
        const outputPath = `./output/cube_frame_${frame}.png`;
        const result = await getCubeFrame(filePath, frame, outputPath);
        if (abortRef.current === seq) {
          onFrameChange?.(result.output_path, frame);
          setLoading(false);
        }
      } catch {
        setLoading(false);
      }

      if (frame === totalFrames - 1) {
        playingRef.current = false;
        setPlaying(false);
        return;
      }

      if (playingRef.current) {
        setTimeout(playNext, 50);
      }
    };

    playNext();
  }, [playing, currentFrame, totalFrames, getCubeFrame, filePath, onFrameChange]);

  if (totalFrames <= 1) return null;

  return (
    <div className="bg-zinc-950/50 rounded-lg border border-purple-500/20 p-3">
      <div className="flex items-center justify-between mb-2">
        <span className="text-[10px] font-semibold text-purple-400 uppercase tracking-wider flex items-center gap-1.5">
          <Film size={11} />
          Frame Navigation
        </span>
        <span className="text-[10px] font-mono text-zinc-500">
          {currentFrame + 1} / {totalFrames}
          {loading && <Loader2 size={10} className="inline ml-1 animate-spin" />}
        </span>
      </div>

      <input
        type="range"
        min={0}
        max={totalFrames - 1}
        value={currentFrame}
        onChange={handleSlider}
        className="w-full accent-purple-500 mb-2"
      />

      <div className="flex items-center justify-center gap-2">
        <button
          onClick={handlePrev}
          disabled={currentFrame === 0 || loading}
          className="p-1.5 rounded-md transition-colors text-zinc-500 hover:text-zinc-200 hover:bg-zinc-800 disabled:opacity-30 disabled:cursor-not-allowed"
        >
          <SkipBack size={14} />
        </button>
        <button
          onClick={togglePlay}
          disabled={loading && !playing}
          className="p-1.5 rounded-md transition-colors hover:bg-zinc-800"
          style={{ color: playing ? "#c084fc" : "#71717a" }}
        >
          {playing ? <Pause size={14} /> : <Play size={14} />}
        </button>
        <button
          onClick={handleNext}
          disabled={currentFrame === totalFrames - 1 || loading}
          className="p-1.5 rounded-md transition-colors text-zinc-500 hover:text-zinc-200 hover:bg-zinc-800 disabled:opacity-30 disabled:cursor-not-allowed"
        >
          <SkipForward size={14} />
        </button>
      </div>
    </div>
  );
}

export default memo(CubeFrameNavInner);
