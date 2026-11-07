const ELLIPSIS = "…";

export function middleEllipsis(name: string, max: number): string {
  if (name.length <= max) return name;
  const budget = Math.max(0, max - ELLIPSIS.length);
  const tail = Math.ceil(budget / 2);
  return `${name.slice(0, budget - tail)}${ELLIPSIS}${tail > 0 ? name.slice(-tail) : ""}`;
}
