export type StorageEventSource = (listener: () => void) => () => void;

export const STORAGE_REFRESH_DELAY_MS = 750;

export function watchStorage(refresh: () => void, sources: readonly StorageEventSource[], delayMs = STORAGE_REFRESH_DELAY_MS): () => void {
  let timer: ReturnType<typeof setTimeout> | null = null;
  const schedule = () => {
    if (timer !== null) clearTimeout(timer);
    timer = setTimeout(() => {
      timer = null;
      refresh();
    }, delayMs);
  };
  const unsubscribes = sources.map((subscribe) => subscribe(schedule));
  return () => {
    for (const unsubscribe of unsubscribes) unsubscribe();
    if (timer !== null) clearTimeout(timer);
    timer = null;
  };
}
