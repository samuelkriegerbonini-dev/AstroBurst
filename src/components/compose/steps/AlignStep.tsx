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
import { ALIGN_REF_AUTO, ALIGN_REF_AUTO_LABEL, alignChannelNote, alignReferenceIndex } from "../../../utils/alignNotes";

function newAlignRunToken(): string {
  return `${Date.now().toString(36)}${Math.floor(Math.random() * 46656).toString(36)}`;
}

interface AlignStepProps {
  state: WizardState;
  onAligned: (paths: Record<string, string>) => void;
}

export default function AlignStep({ state, onAligned }: AlignStepProps) {
  const methodId = useId();
  const refId = useId();
  const [method, setMethod] = useState("phase_correlation");
  const { alignRun, startAlignRun, finishAlignRun, getState, dispatch } = useComposeWizardContext();
  const loading = alignRun?.running ?? false;

  const activeBins = useMemo(
    () => state.bins.filter((b) => b.files.length > 0),
    [state.bins],
  );

  const channelPaths = useMemo(() => alignInputs(state), [state]);
  const discards = discardsNotice("Running Align", rerunDiscards(state, "align"));
  const referenceIndex = alignReferenceIndex(channelPaths, state.alignRefChoice);

  const [runState, setRunState] = useState(
    () => nextAlignedRunState(null, channelPaths, state.alignedPaths, loading, state.alignRefBinId),
  );
  const nextRunState = nextAlignedRunState(runState, channelPaths, state.alignedPaths, loading, state.alignRefBinId);
  if (nextRunState !== runState) setRunState(nextRunState);
  const alignedRun = nextRunState.run;

  const inputPaths = useMemo(() => channelPaths.map((c) => c.path), [channelPaths]);
  const { result, error } = alignRunOutcome(alignRun, inputPaths, alignedRun !== null);
  const refIndex = result ? result.reference_index ?? 0 : referenceIndex;
  const warnings = result?.warnings ?? [];

  const binLabels = useMemo(
    () => Object.fromEntries(activeBins.map((b) => [b.id, b.shortLabel])),
    [activeBins],
  );
  const refBinId = refIndex === null ? undefined : channelPaths[refIndex]?.binId;
  const refLabel = refBinId === undefined ? undefined : binLabels[refBinId] ?? refBinId;

  const handleRefChange = useCallback(
    (value: string) => {
      dispatch({ type: "UPDATE", partial: { alignRefChoice: value === ALIGN_REF_AUTO ? null : value } });
    },
    [dispatch],
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
      res = await alignChannels(paths, dir, method, binIds, referenceIndex, newAlignRunToken());
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
    dispatch({
      type: "UPDATE",
      partial: {
        alignRefBinId: binIds[res.reference_index ?? 0] ?? null,
        alignRunToken: res.run_token ?? null,
      },
    });
  }, [channelPaths, loading, method, referenceIndex, onAligned, startAlignRun, finishAlignRun, getState, dispatch]);

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

        <div className="flex items-center justify-between">
          <label htmlFor={refId} className="text-xs text-zinc-400">Reference</label>
          <select
            id={refId}
            data-testid="align-ref-select"
            value={referenceIndex === null ? ALIGN_REF_AUTO : channelPaths[referenceIndex].binId}
            onChange={(e) => handleRefChange(e.target.value)}
            disabled={loading}
            className="ab-select"
          >
            <option value={ALIGN_REF_AUTO}>{ALIGN_REF_AUTO_LABEL}</option>
            {channelPaths.map((c) => (
              <option key={c.binId} value={c.binId}>{binLabels[c.binId] ?? c.binId}</option>
            ))}
          </select>
        </div>

        <div className="flex flex-col gap-1">
          <span className="text-[9px] text-zinc-600 uppercase tracking-wider">Channels to align</span>
          {channelPaths.map((c, i) => {
            const bin = activeBins.find((b) => b.id === c.binId);
            const ch = result?.channels?.[i];
            const offset = ch?.offset;
            const requested = result?.align_method ?? method;
            const isReference = i === refIndex;
            const outcome = alignChannelOutcome(ch, requested, isReference);
            const note = alignChannelNote(ch, bin?.shortLabel ?? c.binId, requested, isReference, result?.reference_rule, refLabel);
            const match = alignMatchSummary(ch);
            return (
              <div key={c.binId} data-bin={c.binId} className="flex flex-col py-1">
                <div className="flex items-center justify-between gap-2">
                  <div className="flex items-center gap-1.5">
                    <span className="w-2 h-2 rounded-full" style={{ background: bin?.color }} />
                    <span className="text-[10px] text-zinc-300">{bin?.shortLabel}</span>
                    {isReference && <span data-testid="align-ref-tag" className="text-[8px] text-sky-400/60 ml-1">REF</span>}
                    {outcome.usedMethod && <span className="text-[8px] text-zinc-500">via {outcome.usedMethod}</span>}
                  </div>
                  <div className="flex items-center gap-2">
                    {match && (
                      <span className="text-[9px] font-mono text-sky-400/60">{match}</span>
                    )}
                    {offset && !isReference && !note?.wcs && (
                      <span className="text-[10px] font-mono text-zinc-400 whitespace-pre" title={ALIGN_OFFSET_TITLE}>
                        {formatAlignOffset(offset)}
                      </span>
                    )}
                  </div>
                </div>
                {note && (
                  <span
                    data-testid="align-channel-note"
                    className={`text-[9px] whitespace-pre-wrap mt-0.5 ${note.wcs ? "text-zinc-400" : "text-amber-400/90"}`}
                    title={note.wcs ? ALIGN_OFFSET_TITLE : undefined}
                  >
                    {note.text}
                  </span>
                )}
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
        {warnings.length > 0 && (
          <div data-testid="align-warnings" className="flex flex-col gap-0.5 text-[9px] text-amber-400/90">
            {warnings.map((w, i) => (
              <p key={i}>{w}</p>
            ))}
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
