export function quietCompletion(quietAdd: boolean, batchComplete: boolean, quietInFlight: boolean): boolean {
  return quietAdd && (batchComplete || quietInFlight);
}
