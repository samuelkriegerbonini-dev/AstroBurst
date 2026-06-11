import { memo, useEffect } from "react";
import { formatThroughput } from "../../utils/format";
import { useTimer } from "../../hooks/useTimer";
import type { QueueStats } from "../../shared/types";

interface StatsBarProps {
  stats: QueueStats;
  isProcessing: boolean;
  isComplete: boolean;
}

function StatsBar({ stats, isProcessing, isComplete }: StatsBarProps) {
  const timer = useTimer();
  const { start, stop } = timer;

  useEffect(() => {
    if (isProcessing) start();
    else stop();
  }, [isProcessing, start, stop]);

  const throughput = stats.totalBytes > 0 && timer.elapsed > 0
    ? formatThroughput(stats.totalBytes, timer.elapsed)
    : null;

  return (
    <div className="flex items-center justify-between text-[11px]">
      <div className="flex items-center gap-2.5 text-zinc-400 font-mono">
        <span>{stats.total} files</span>
        <span style={{ color: "rgba(20,184,166,0.25)" }}>|</span>
        <span style={{ color: "var(--ab-green)" }}>{stats.done} done</span>
        {stats.failed > 0 && (
          <>
            <span style={{ color: "rgba(20,184,166,0.25)" }}>|</span>
            <span className="text-red-400">{stats.failed} err</span>
          </>
        )}
        <span style={{ color: "rgba(20,184,166,0.25)" }}>|</span>
        <span className="text-zinc-300">{timer.formatted}</span>
      </div>

      <div className="flex items-center gap-2.5 text-zinc-500 font-mono">
        {isComplete && (
          <span className="font-medium animate-fade-in cosmic-text">
            Complete
          </span>
        )}
        {throughput && <span>{throughput}</span>}
      </div>
    </div>
  );
}

export default memo(StatsBar);
