export interface TimedNotice<T> {
  show: (value: T) => void;
  clear: () => void;
}

export function createTimedNotice<T>(onChange: (value: T | null) => void, durationMs: number): TimedNotice<T> {
  let timer: ReturnType<typeof setTimeout> | null = null;
  const stop = () => {
    if (timer !== null) clearTimeout(timer);
    timer = null;
  };
  return {
    show: (value) => {
      stop();
      onChange(value);
      timer = setTimeout(() => {
        timer = null;
        onChange(null);
      }, durationMs);
    },
    clear: () => {
      stop();
      onChange(null);
    },
  };
}
