export const FOCUSABLE_SELECTOR =
  'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

export function focusTrapTarget(count: number, activeIndex: number, backwards: boolean): number | null {
  if (count <= 0) return null;
  if (backwards) return activeIndex <= 0 ? count - 1 : null;
  return activeIndex < 0 || activeIndex >= count - 1 ? 0 : null;
}
