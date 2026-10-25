import { memo, useMemo, useSyncExternalStore } from "react";
import { X } from "lucide-react";
import ProgressBar from "../file/ProgressBar";
import { formatBytes } from "../archive/logic/productView";
import { sampleActions, sampleControls, sampleDataStore } from "../../hooks/useSampleData";
import { CUBE_SAMPLE_SETS, jwstSampleTerms, openSampleProducts, sampleOfferAction, samplePercent } from "../../utils/sampleOffers";
import { createFocusHandoff } from "../../utils/focusHandoff";

const OFFER_STYLE = {
  background: "color-mix(in srgb, var(--ab-violet) 8%, transparent)",
  border: "1px solid color-mix(in srgb, var(--ab-violet) 20%, transparent)",
  color: "var(--ab-violet)",
};

function CubeSampleOffer({ id, idle }: { id: string; idle: boolean }) {
  const entries = useSyncExternalStore(sampleDataStore.subscribe, sampleDataStore.get, sampleDataStore.get);
  const actions = useMemo(() => sampleActions(sampleDataStore, id, openSampleProducts), [id]);
  const focus = useMemo(() => createFocusHandoff(), []);
  const entry = entries.find((e) => e.id === id);
  const sample = sampleControls(entry, actions);
  if (!entry || !sample || (!idle && sample.state !== "downloading" && !sample.error)) return null;
  const action = sampleOfferAction(sample);
  const percent = samplePercent(sample);
  const ActionIcon = action?.icon;
  const terms = jwstSampleTerms(id);
  return (
    <div data-cube-sample={id} data-sample-state={sample.state} className="w-full px-3 flex flex-col gap-1 text-[10px] text-zinc-500">
      <span className="text-zinc-300">{sample.title}</span>
      {entry.demonstrates && <span className="leading-snug">{entry.demonstrates}</span>}
      {action && ActionIcon && (
        <button
          type="button"
          data-cube-sample-action
          ref={focus.arriving}
          onClick={action.run}
          className="self-start flex items-center gap-1.5 px-3 py-1.5 rounded-md text-[10px] font-medium transition-all"
          style={OFFER_STYLE}
        >
          <ActionIcon size={10} />
          {action.label}
        </button>
      )}
      {sample.state === "downloading" && (
        <div className="flex items-center gap-2">
          <div
            role="progressbar"
            aria-label={`${sample.title} download`}
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={percent}
            aria-valuetext={`${formatBytes(sample.receivedBytes)} of ${formatBytes(sample.totalBytes)}`}
            data-cube-sample-progress
            className="w-32"
            title={sample.file ?? undefined}
          >
            <ProgressBar value={percent} />
          </div>
          <span className="font-mono tabular-nums">{percent}%</span>
          <button
            type="button"
            data-cube-sample-cancel
            ref={focus.leaving}
            onClick={() => void sample.cancel()}
            className="flex items-center gap-1 text-zinc-500 hover:text-zinc-300"
            aria-label={`Cancel the ${sample.title} download`}
          >
            <X size={10} />
            Cancel
          </button>
        </div>
      )}
      {sample.error && (
        <span role="alert" className="text-red-400 break-words">
          {sample.error}
        </span>
      )}
      <span className="text-zinc-600 leading-snug">{sample.credit}</span>
      {terms && <span data-cube-sample-terms className="text-zinc-500 leading-snug">{terms}</span>}
      {sample.licence && <span className="text-zinc-600 leading-snug">{sample.licence}</span>}
    </div>
  );
}

function CubeSampleOffers({ idle }: { idle: boolean }) {
  return (
    <>
      {CUBE_SAMPLE_SETS.map((id) => (
        <CubeSampleOffer key={id} id={id} idle={idle} />
      ))}
    </>
  );
}

export default memo(CubeSampleOffers);
