export function formatCount(value: number): string {
  if (!Number.isFinite(value)) return "—";
  const rounded = Math.round(value);
  const grouped = Math.abs(rounded).toString().replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  return rounded < 0 ? `-${grouped}` : grouped;
}
