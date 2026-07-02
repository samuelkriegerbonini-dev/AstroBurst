import { memo, useCallback, useState, useRef, useEffect } from "react";

interface SliderProps {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  scale?: "linear" | "log";
  disabled?: boolean;
  accent?: string;
  format?: (v: number) => string;
  onChange: (v: number) => void;
  hint?: string;
}

const LOG_STEPS = 1000;

function Slider({
  label,
  value,
  min,
  max,
  step,
  scale = "linear",
  disabled = false,
  accent = "teal",
  format,
  onChange,
  hint,
}: SliderProps) {
  const [editing, setEditing] = useState(false);
  const [editText, setEditText] = useState("");
  const editStartText = useRef("");
  const inputRef = useRef<HTMLInputElement>(null);

  const isLog = scale === "log" && min > 0 && max > min;
  const logRange = isLog ? Math.log(max / min) : 1;
  const toPos = useCallback(
    (v: number) => Math.log(Math.max(min, Math.min(max, v)) / min) / logRange,
    [isLog, min, max, logRange],
  );
  const fromPos = useCallback((p: number) => min * Math.exp(p * logRange), [min, logRange]);

  const handleChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const raw = parseFloat(e.target.value);
      onChange(isLog ? fromPos(raw) : raw);
    },
    [onChange, isLog, fromPos],
  );

  const display = format ? format(value) : String(value);
  const pct = isLog ? toPos(value) * 100 : ((value - min) / (max - min)) * 100;

  const handleValueClick = useCallback(() => {
    if (disabled) return;
    setEditing(true);
    setEditText(display);
    editStartText.current = display;
  }, [disabled, display]);

  const commitEdit = useCallback(() => {
    setEditing(false);
    if (editText === editStartText.current) return;
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
        min={isLog ? 0 : min}
        max={isLog ? 1 : max}
        step={isLog ? 1 / LOG_STEPS : step}
        value={isLog ? toPos(value) : value}
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
