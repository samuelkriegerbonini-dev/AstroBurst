import { memo, useCallback, useState, useRef, useEffect, useId } from "react";
import { resolveTypedValue } from "./sliderValue";

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
  onChange?: (v: number) => void;
  onCommit?: (v: number) => void;
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
  onCommit,
  hint,
}: SliderProps) {
  const rangeId = useId();
  const [editing, setEditing] = useState(false);
  const [editText, setEditText] = useState("");
  const editStartText = useRef("");
  const inputRef = useRef<HTMLInputElement>(null);
  const rangeRef = useRef<HTMLInputElement>(null);
  const [dragValue, setDragValue] = useState<number | null>(null);
  const dragValueRef = useRef<number | null>(null);
  const onCommitRef = useRef(onCommit);
  onCommitRef.current = onCommit;

  const isLog = scale === "log" && min > 0 && max > min;
  const logRange = isLog ? Math.log(max / min) : 1;
  const toPos = useCallback(
    (v: number) => Math.log(Math.max(min, Math.min(max, v)) / min) / logRange,
    [min, max, logRange],
  );
  const fromPos = useCallback((p: number) => min * Math.exp(p * logRange), [min, logRange]);

  const handleChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const raw = parseFloat(e.target.value);
      const v = isLog ? fromPos(raw) : raw;
      if (onCommitRef.current) {
        dragValueRef.current = v;
        setDragValue(v);
      }
      onChange?.(v);
    },
    [onChange, isLog, fromPos],
  );

  useEffect(() => {
    const el = rangeRef.current;
    if (!el) return;
    const fire = () => {
      const v = dragValueRef.current;
      dragValueRef.current = null;
      setDragValue(null);
      if (v !== null) onCommitRef.current?.(v);
    };
    el.addEventListener("change", fire);
    return () => el.removeEventListener("change", fire);
  }, []);

  const effective = dragValue ?? value;
  const display = format ? format(effective) : String(effective);
  const pct = isLog ? toPos(effective) * 100 : ((effective - min) / (max - min)) * 100;

  const handleValueClick = useCallback(() => {
    if (disabled) return;
    setEditing(true);
    const raw = Number.isInteger(effective) ? String(effective) : String(Number(effective.toFixed(6)));
    setEditText(raw);
    editStartText.current = raw;
  }, [disabled, effective]);

  const commitEdit = useCallback(() => {
    setEditing(false);
    if (editText === editStartText.current) return;
    const parsed = parseFloat(editText);
    if (!isNaN(parsed)) {
      (onCommit ?? onChange)?.(resolveTypedValue(parsed, min, max, step, isLog));
    }
  }, [editText, min, max, step, isLog, onChange, onCommit]);

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
        <label className="ab-slider-label" htmlFor={rangeId}>
          {label}
          {hint && <span className="ab-slider-hint">{hint}</span>}
        </label>
        {editing ? (
          <input
            ref={inputRef}
            type="text"
            aria-label={`${label} value`}
            value={editText}
            onChange={(e) => setEditText(e.target.value)}
            onBlur={commitEdit}
            onKeyDown={handleEditKeyDown}
            className="ab-slider-value-edit"
          />
        ) : (
          <button
            type="button"
            className="ab-slider-value"
            onClick={handleValueClick}
            disabled={disabled}
            aria-label={`Edit ${label} value`}
            title="Click to edit value"
          >
            {display}
          </button>
        )}
      </div>
      <input
        ref={rangeRef}
        id={rangeId}
        type="range"
        min={isLog ? 0 : min}
        max={isLog ? 1 : max}
        step={isLog ? 1 / LOG_STEPS : step}
        value={isLog ? toPos(effective) : effective}
        aria-valuetext={display}
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
