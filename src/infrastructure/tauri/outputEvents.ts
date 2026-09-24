type OutputListener = (result: unknown) => void;

const listeners = new Set<OutputListener>();

export function onCommandOutputs(listener: OutputListener): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function announceCommandOutputs(result: unknown): void {
  for (const listener of listeners) listener(result);
}
