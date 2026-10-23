export function createTimedNotice<T>(onChange: (value: T | null) => void, durationMs: number): (value: T) => void {
  let timer: ReturnType<typeof setTimeout> | null = null;
  return (value) => {
    if (timer !== null) clearTimeout(timer);
    onChange(value);
    timer = setTimeout(() => {
      timer = null;
      onChange(null);
    }, durationMs);
  };
}
