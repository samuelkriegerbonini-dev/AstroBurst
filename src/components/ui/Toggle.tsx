import { memo, useCallback } from "react";

interface ToggleProps {
  label: string;
  checked: boolean;
  disabled?: boolean;
  accent?: string;
  badge?: string | null;
  onChange: (v: boolean) => void;
}

function Toggle({ label, checked, disabled = false, accent = "teal", badge, onChange }: ToggleProps) {
  const handleClick = useCallback(() => {
    if (!disabled) onChange(!checked);
  }, [disabled, checked, onChange]);

  return (
    <div className="flex items-center justify-between py-0.5">
      <div className="flex items-center gap-1.5">
        <label className="text-xs text-zinc-400">{label}</label>
        {badge && (
          <span className="text-[9px] text-emerald-400 bg-emerald-900/30 px-1.5 py-0.5 rounded font-medium">
            {badge}
          </span>
        )}
      </div>
      <button
        type="button"
        role="switch"
        aria-checked={checked}
        onClick={handleClick}
        disabled={disabled}
        className="ab-toggle"
        data-accent={accent}
        data-checked={checked}
      >
        <span className="ab-toggle-thumb" data-checked={checked} />
      </button>
    </div>
  );
}

export default memo(Toggle);
