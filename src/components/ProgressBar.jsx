import { motion } from "framer-motion";

const variantColors = {
  blue: "bg-blue-500",
  green: "bg-green-500",
  red: "bg-red-500",
};

const variantGlows = {
  blue: "shadow-[0_0_8px_rgba(59,130,246,0.5)]",
  green: "shadow-[0_0_8px_rgba(34,197,94,0.5)]",
  red: "shadow-[0_0_8px_rgba(239,68,68,0.5)]",
};

export default function ProgressBar({
  value = 0,
  variant = "blue",
  indeterminate = false,
  height = "h-1",
  className = "",
}) {
  const colorClass = variantColors[variant] || variantColors.blue;
  const glowClass = variantGlows[variant] || variantGlows.blue;

  if (indeterminate) {
    return (
      <div
        className={`${height} w-full bg-zinc-800 rounded-full overflow-hidden ${className}`}
      >
        <div
          className={`h-full w-1/3 ${colorClass} rounded-full animate-shimmer`}
        />
      </div>
    );
  }

  return (
    <div
      className={`${height} w-full bg-zinc-800 rounded-full overflow-hidden ${className}`}
    >
      <motion.div
        className={`h-full ${colorClass} rounded-full ${glowClass}`}
        initial={{ width: 0 }}
        animate={{ width: `${Math.min(100, Math.max(0, value))}%` }}
        transition={{ type: "spring", stiffness: 100, damping: 20 }}
      />
    </div>
  );
}
