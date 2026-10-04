import { useState, useCallback, useId, useMemo } from "react";
import type { WizardState } from "../wizard";
import {
  ALIGN_OFFSET_TITLE,
  alignChannelOutcome,
  alignInputs,
  alignMatchSummary,
  alignRunFinish,
  alignRunMethodLabel,
  alignRunOutcome,
  discardsNotice,
  formatAlignOffset,
  nextAlignedRunState,
  rerunDiscards,
} from "../../../utils/wizard";
import { alignChannels } from "../../../services/compose";
import type { AlignResult } from "../../../shared/types/compose";
import { getOutputDir } from "../../../infrastructure/tauri";
import { useComposeWizardContext, type WizardAlignRun } from "../../../context/ComposeWizardContext";
import { RunButton } from "../../ui";
import AlignPreview from "../AlignPreview";

interface AlignStepProps {
  state: WizardState;
  onAligned: (paths: Record<string, string>) => void;
}

export default function AlignStep({ state, onAligned }: AlignStepProps) {
  const methodId = useId();
  const [method, setMethod] = useState("phase_correlation");
  const { alignRun, startAlignRun, finishAlignRun, getState } = useComposeWizardContext();
  const loading = alignRun?.running ?? false;

  const activeBins = useMemo(
    () => state.bins.filter((b) => b.files.length > 0),
    [state.bins],
  );

  const channelPaths = useMemo(() => alignInputs(state), [state]);
  const discards = discardsNotice("Running Align", rerunDiscards(state, "align"));

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
    const started = channelPaths;
    const paths = started.map((c) => c.path);
    const binIds = started.map((c) => c.binId);
    const startRecord: WizardAlignRun = { running: true, inputs: paths, result: null, error: "" };
    startAlignRun(startRecord);
    let res: AlignResult | null = null;
    let error = "";
    try {
      const dir = await getOutputDir();
      res = await alignChannels(paths, dir, method, binIds);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
    const outcome = alignRunFinish(started, alignInputs(getState()), res, error);
    if (!finishAlignRun(startRecord, outcome.record) || !outcome.store || !res?.channels) return;
    const aligned: Record<string, string> = {};
    res.channels.forEach((ch, i) => {
      const key = ch.cache_key || ch.path;
      if (started[i] && key) aligned[started[i].binId] = key;
    });
    onAligned(aligned);
  }, [channelPaths, loading, method, onAligned, startAlignRun, finishAlignRun, getState]);

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
        {discards && !loading && <div className="text-[9px] text-amber-400/80">{discards}</div>}

        {result && (
          <div className="text-[9px] text-zinc-500">
            {alignRunMethodLabel(result)}, {result.dimensions?.[0]}x{result.dimensions?.[1]}, {result.elapsed_ms}ms
          </div>
        )}
        {error && <div className="text-[9px] text-red-400">{error}</div>}
      </div>

      <div className="flex flex-col flex-[2] basis-[300px] min-w-[240px]">
        <AlignPreview run={alignedRun} labels={binLabels} aligning={loading} />
      </div>
    </div>
  );
}
