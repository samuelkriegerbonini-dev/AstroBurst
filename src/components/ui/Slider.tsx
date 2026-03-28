import { memo, useCallback, useState, useRef, useEffect } from "react";

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
  hint?: string;
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
  hint,
}: SliderProps) {
  const [editing, setEditing] = useState(false);
  const [editText, setEditText] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  const handleChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => onChange(parseFloat(e.target.value)),
    [onChange],
  );

  const display = format ? format(value) : String(value);
  const pct = ((value - min) / (max - min)) * 100;

  const handleValueClick = useCallback(() => {
    if (disabled) return;
    setEditing(true);
    setEditText(display);
  }, [disabled, display]);

  const commitEdit = useCallback(() => {
    setEditing(false);
    const parsed = parseFloat(editText);
    if (!isNaN(parsed)) {
      const clamped = Math.max(min, Math.min(max, parsed));
      onChange(clamped);
    }
  }, [editText, min, max, onChange]);

  const handleEditKeyDown = useCallback((e: React.KeyboardEvent) => {
    if (e.key === "Enter") { e.preventDefault(); commitEdit(); }
    if (e.key === "Escape") { setEditing(false); }
  }, [commitEdit]);

  useEffect(() => {
    if (editing && inputRef.current) {
      inputRef.current.select();
    }
  }, [editing]);

  return (
    <div className="ab-slider-group">
      <div className="flex justify-between items-center mb-1">
        <label className="ab-slider-label">
          {label}
          {hint && <span className="ab-slider-hint">{hint}</span>}
        </label>
        {editing ? (
          <input
            ref={inputRef}
            type="text"
            value={editText}
            onChange={(e) => setEditText(e.target.value)}
            onBlur={commitEdit}
            onKeyDown={handleEditKeyDown}
            className="ab-slider-value-edit"
          />
        ) : (
          <span
            className="ab-slider-value"
            onClick={handleValueClick}
            title="Click to edit value"
          >
            {display}
          </span>
        )}
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
