import { useState, useCallback, useId, useMemo } from "react";
import type { WizardState } from "../wizard";
import { resolveChannelPath as resolveWizardPath } from "../wizard";
import {
  ALIGN_OFFSET_TITLE,
  alignChannelOutcome,
  alignMatchSummary,
  alignRunOutcome,
  formatAlignOffset,
  nextAlignedRunState,
} from "../../../utils/wizard";
import { alignChannels } from "../../../services/compose";
import { getOutputDir } from "../../../infrastructure/tauri";
import { useComposeWizardContext } from "../../../context/ComposeWizardContext";
import { RunButton } from "../../ui";
import AlignPreview from "../AlignPreview";

interface AlignStepProps {
  state: WizardState;
  onAligned: (paths: Record<string, string>) => void;
}

function resolveChannelPath(state: WizardState, binId: string): string | null {
  return resolveWizardPath(state, binId, "stacked");
}

export default function AlignStep({ state, onAligned }: AlignStepProps) {
  const methodId = useId();
  const [method, setMethod] = useState("phase_correlation");
  const { alignRun, setAlignRun } = useComposeWizardContext();
  const loading = alignRun?.running ?? false;

  const activeBins = useMemo(
    () => state.bins.filter((b) => b.files.length > 0),
    [state.bins],
  );

  const channelPaths = useMemo(() => {
    const entries: { binId: string; path: string }[] = [];
    for (const bin of activeBins) {
      const p = resolveChannelPath(state, bin.id);
      if (p) entries.push({ binId: bin.id, path: p });
    }
    return entries;
  }, [activeBins, state]);

  const [runState, setRunState] = useState(
    () => nextAlignedRunState(null, channelPaths, state.alignedPaths, loading),
  );
  const nextRunState = nextAlignedRunState(runState, channelPaths, state.alignedPaths, loading);
  if (nextRunState !== runState) setRunState(nextRunState);
  const alignedRun = nextRunState.run;

  const inputPaths = useMemo(() => channelPaths.map((c) => c.path), [channelPaths]);
  const { result, error } = alignRunOutcome(alignRun, inputPaths, alignedRun !== null);

  const binLabels = useMemo(
    () => Object.fromEntries(activeBins.map((b) => [b.id, b.shortLabel])),
    [activeBins],
  );

  const handleAlign = useCallback(async () => {
    if (channelPaths.length < 2 || loading) return;
    const paths = channelPaths.map((c) => c.path);
    const binIds = channelPaths.map((c) => c.binId);
    setAlignRun({ running: true, inputs: paths, result: null, error: "" });
    try {
      const dir = await getOutputDir();
      const res = await alignChannels(paths, dir, method, binIds);
      const aligned: Record<string, string> = {};
      res.channels?.forEach((ch, i) => {
        const key = ch.cache_key || ch.path;
        if (channelPaths[i] && key) aligned[channelPaths[i].binId] = key;
      });
      setAlignRun({ running: false, inputs: paths, result: res, error: "" });
      if (res.channels) onAligned(aligned);
    } catch (e) {
      setAlignRun({ running: false, inputs: paths, result: null, error: e instanceof Error ? e.message : String(e) });
    }
  }, [channelPaths, loading, method, onAligned, setAlignRun]);

  if (channelPaths.length < 2) {
    return (
      <div className="flex items-center justify-center py-12 text-zinc-600 text-xs">
        Need at least 2 channels to align.
      </div>
    );
  }

  return (
    <div className="flex flex-wrap items-stretch gap-3 p-3 min-h-full">
      <div className="flex flex-col gap-3 flex-1 basis-[260px] min-w-[240px] max-w-[420px]">
        <div className="flex items-center justify-between">
          <label htmlFor={methodId} className="text-xs text-zinc-400">Method</label>
          <select
            id={methodId}
            value={method}
            onChange={(e) => setMethod(e.target.value)}
            disabled={loading}
            className="ab-select"
          >
            <option value="phase_correlation">Phase Correlation (sub-pixel)</option>
            <option value="affine">Star-based Affine (rotation)</option>
          </select>
        </div>

        <div className="flex flex-col gap-1">
          <span className="text-[9px] text-zinc-600 uppercase tracking-wider">Channels to align</span>
          {channelPaths.map((c, i) => {
            const bin = activeBins.find((b) => b.id === c.binId);
            const ch = result?.channels?.[i];
            const offset = ch?.offset;
            const outcome = alignChannelOutcome(ch, result?.align_method ?? method, i === 0);
            const match = alignMatchSummary(ch);
            return (
              <div key={c.binId} className="flex flex-col py-1">
                <div className="flex items-center justify-between gap-2">
                  <div className="flex items-center gap-1.5">
                    <span className="w-2 h-2 rounded-full" style={{ background: bin?.color }} />
                    <span className="text-[10px] text-zinc-300">{bin?.shortLabel}</span>
                    {i === 0 && <span className="text-[8px] text-sky-400/60 ml-1">REF</span>}
                    {outcome.usedMethod && <span className="text-[8px] text-zinc-500">via {outcome.usedMethod}</span>}
                  </div>
                  <div className="flex items-center gap-2">
                    {match && (
                      <span className="text-[9px] font-mono text-sky-400/60">{match}</span>
                    )}
                    {offset && i > 0 && (
                      <span className="text-[10px] font-mono text-zinc-400 whitespace-pre" title={ALIGN_OFFSET_TITLE}>
                        {formatAlignOffset(offset)}
                      </span>
                    )}
                  </div>
                </div>
                {outcome.unregistered && (
                  <span className="text-[9px] text-amber-400/90 bg-amber-500/10 border border-amber-500/20 rounded px-1.5 py-0.5 mt-0.5">
                    {outcome.unregistered}
                  </span>
                )}
              </div>
            );
          })}
        </div>

        <RunButton
          label="Align Channels"
          runningLabel="Aligning..."
          running={loading}
          disabled={channelPaths.length < 2}
          accent="sky"
          onClick={handleAlign}
        />

        {result && (
          <div className="text-[9px] text-zinc-500">
            {result.align_method}, {result.dimensions?.[0]}x{result.dimensions?.[1]}, {result.elapsed_ms}ms
          </div>
        )}
        {error && <div className="text-[9px] text-red-400">{error}</div>}
      </div>

      <div className="flex flex-col flex-[2] basis-[300px] min-w-[260px]">
        <AlignPreview run={alignedRun} labels={binLabels} aligning={loading} />
      </div>
    </div>
  );
}
