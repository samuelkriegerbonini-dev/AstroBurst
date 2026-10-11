import { useEffect, useState, type InputHTMLAttributes } from "react";
import {
  decimalDraftFor,
  decimalDraftText,
  decimalEditValue,
  decimalFieldState,
  type DecimalFieldOptions,
} from "../../utils/decimalText";

type NativeInputProps = Omit<
  InputHTMLAttributes<HTMLInputElement>,
  "type" | "value" | "defaultValue" | "onChange" | "inputMode" | "min" | "max" | "step"
>;

export interface DecimalFieldProps extends NativeInputProps, DecimalFieldOptions {
  value: string;
  onChange: (text: string) => void;
  error?: string | null;
}

const INVALID_RING_CLASS = "ring-1 ring-red-500/70";

export default function DecimalField({
  value,
  onChange,
  min,
  max,
  integer,
  optional,
  error,
  className,
  title,
  ...inputProps
}: DecimalFieldProps) {
  const shownError = decimalFieldState(value, { min, max, integer, optional }).error ?? error ?? null;
  const invalid = shownError !== null;
  return (
    <input
      {...inputProps}
      type="text"
      inputMode="decimal"
      value={value}
      onChange={(e) => onChange(e.target.value)}
      aria-invalid={invalid ? true : undefined}
      title={invalid ? shownError : title}
      className={invalid ? [className, INVALID_RING_CLASS].filter(Boolean).join(" ") : className}
    />
  );
}

export interface DecimalNumberFieldProps extends Omit<DecimalFieldProps, "value" | "onChange"> {
  value: number | null;
  onValueChange: (value: number | null) => void;
}

export function DecimalNumberField({ value, onValueChange, min, max, integer, optional, ...fieldProps }: DecimalNumberFieldProps) {
  const [draft, setDraft] = useState(() => decimalDraftText(value));

  useEffect(() => {
    setDraft((current) => decimalDraftFor(current, value, { min, max, integer, optional }));
  }, [value, min, max, integer, optional]);

  const handleChange = (text: string) => {
    setDraft(text);
    const next = decimalEditValue(text, value, { min, max, integer, optional });
    if (next !== undefined) onValueChange(next);
  };

  return (
    <DecimalField
      {...fieldProps}
      min={min}
      max={max}
      integer={integer}
      optional={optional}
      value={draft}
      onChange={handleChange}
    />
  );
}
