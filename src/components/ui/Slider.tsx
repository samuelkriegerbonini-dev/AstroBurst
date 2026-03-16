import { memo, useCallback } from "react";

interface SliderProps {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  disabled?: boolean;
  accent?: string;
  format?: (v: number) => string;
  onChange: (v: number) => void;
}

function Slider({
  label,
  value,
  min,
  max,
  step,
  disabled = false,
  accent = "teal",
  format,
  onChange,
}: SliderProps) {
  const handleChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => onChange(parseFloat(e.target.value)),
    [onChange],
  );

  const display = format ? format(value) : String(value);
  const pct = ((value - min) / (max - min)) * 100;

  return (
    <div className="ab-slider-group">
      <div className="flex justify-between items-center mb-1">
        <label className="text-xs text-zinc-400">{label}</label>
        <span className="ab-slider-value">{display}</span>
      </div>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={handleChange}
        disabled={disabled}
        className="ab-slider"
        data-accent={accent}
        style={{ "--slider-pct": `${pct}%` } as React.CSSProperties}
      />
    </div>
  );
}

export default memo(Slider);
