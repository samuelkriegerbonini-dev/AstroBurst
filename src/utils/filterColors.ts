export const FILTER_COLORS: Readonly<Record<string, string>> = {
  F070W: "#dc2626", F090W: "#ea580c", F115W: "#d97706",
  F140M: "#ca8a04", F150W: "#84cc16", F162M: "#22c55e",
  F164N: "#16a34a", F182M: "#14b8a6", F187N: "#0d9488",
  F200W: "#0ea5e9", F210M: "#2563eb", F212N: "#4f46e5",
  F250M: "#7c3aed", F277W: "#9333ea", F300M: "#a855f7",
  F322W2: "#c026d3", F335M: "#db2777", F356W: "#e11d48",
  F360M: "#f43f5e", F405N: "#ef4444", F410M: "#dc2626",
  F430M: "#b91c1c", F444W: "#991b1b", F460M: "#7f1d1d",
  F466N: "#78350f", F470N: "#713f12", F480M: "#365314",
};

export function getFilterColor(filter?: string | null): string {
  if (!filter) return "var(--ab-text-3)";
  return FILTER_COLORS[filter.toUpperCase().trim()] ?? "var(--ab-text-3)";
}
