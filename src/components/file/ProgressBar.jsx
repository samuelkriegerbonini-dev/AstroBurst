import { memo } from "react";

const variantColors = {
  blue: "cosmic-progress",
  green: "",
  red: "",
};

const variantInline = {
  blue: {},
  green: { background: "linear-gradient(90deg, var(--ab-teal), var(--ab-green))" },
  red: { background: "linear-gradient(90deg, #ef4444, #f87171)" },
};

const variantGlows = {
  blue: "shadow-[0_0_8px_var(--ab-glow-teal)]",
  green: "shadow-[0_0_8px_rgba(16,185,129,0.4)]",
  red: "shadow-[0_0_8px_rgba(239,68,68,0.5)]",
};

function ProgressBar({
  value = 0,
  variant = "blue",
  indeterminate = false,
  height = "h-1",
  className = "",
}) {
  const colorClass = variantColors[variant] || "";
  const glowClass = variantGlows[variant] || variantGlows.blue;
  const inlineStyle = variantInline[variant] || {};
  const clamped = Math.min(100, Math.max(0, value));

  if (indeterminate) {
    return (
      <div className={`${height} w-full rounded-full overflow-hidden ${className}`} style={{ background: "rgba(20,184,166,0.08)" }}>
        <div className={`h-full w-1/3 cosmic-progress rounded-full animate-shimmer`} />
      </div>
    );
  }

  return (
    <div className={`${height} w-full rounded-full overflow-hidden ${className}`} style={{ background: "rgba(20,184,166,0.08)" }}>
      <div
        className={`h-full ${colorClass} rounded-full ${glowClass}`}
        style={{
          width: `${clamped}%`,
          transition: "width 0.3s cubic-bezier(0.4, 0, 0.2, 1)",
          ...inlineStyle,
        }}
      />
    </div>
  );
}

export default memo(ProgressBar);
