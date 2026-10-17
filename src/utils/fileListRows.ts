export function rowTops(heights: readonly number[]): number[] {
  const tops = [0];
  for (const height of heights) tops.push(tops[tops.length - 1] + height);
  return tops;
}

function rowAt(tops: readonly number[], y: number): number {
  let lo = 0;
  let hi = tops.length - 2;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (tops[mid] <= y) lo = mid;
    else hi = mid - 1;
  }
  return lo;
}

export function rowWindow(tops: readonly number[], scrollTop: number, viewHeight: number, overscan: number): { start: number; end: number } {
  const count = tops.length - 1;
  return {
    start: Math.max(0, rowAt(tops, scrollTop) - overscan),
    end: Math.min(count, rowAt(tops, scrollTop + viewHeight) + 1 + overscan),
  };
}
