import { useState } from "react";
import DecimalField from "../ui/DecimalField";
import { axisTextForChannel, channelFromAxisText, parseChannelInput, type PlotMapping } from "../../utils/spectrumRange";

export interface RangeEdgeFieldProps {
  edge: string;
  channel: number | null;
  channelCount: number;
  mapping: PlotMapping | null;
  unit: string;
  channelLabel: string;
  axisLabel: string;
  placeholder: string;
  onCommit: (channel: number) => void;
}

const FIELD_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-1.5 py-0.5 text-[10px] font-mono text-zinc-200 focus:border-violet-500/50 w-14 disabled:opacity-40";

export default function RangeEdgeField({
  edge,
  channel,
  channelCount,
  mapping,
  unit,
  channelLabel,
  axisLabel,
  placeholder,
  onCommit,
}: RangeEdgeFieldProps) {
  const [typed, setTyped] = useState<string | null>(null);
  const [typedFor, setTypedFor] = useState(channel);
  const channelMoved = typedFor !== channel;
  if (channelMoved) {
    setTypedFor(channel);
    setTyped(null);
  }
  const draft = channelMoved ? null : typed;
  const commit = () => {
    if (draft === null) return;
    const next = mapping === null ? parseChannelInput(draft, channelCount) : channelFromAxisText(draft, mapping, unit).channel;
    if (next === null) return;
    setTyped(null);
    if (next !== channel) onCommit(next);
  };
  const handleKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") commit();
    else if (e.key === "Escape") setTyped(null);
  };
  const shared = {
    optional: channel === null,
    placeholder,
    "data-range-edge": edge,
    onChange: setTyped,
    onBlur: commit,
    onKeyDown: handleKeyDown,
    className: FIELD_CLASS,
  };
  if (mapping === null) {
    return (
      <DecimalField
        {...shared}
        integer
        min={0}
        max={Math.max(channelCount - 1, 0)}
        value={draft ?? (channel === null ? "" : String(channel))}
        aria-label={channelLabel}
      />
    );
  }
  return (
    <DecimalField
      {...shared}
      value={draft ?? axisTextForChannel(channel, mapping, unit)}
      aria-label={axisLabel}
      title={channel === null ? undefined : `channel ${channel}`}
      error={draft ? channelFromAxisText(draft, mapping, unit).error : null}
    />
  );
}
