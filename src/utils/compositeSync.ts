import { withVersionParam } from "./processingChain";

export type CompositeChannel = "r" | "g" | "b";

export interface CompositeSyncLedger {
  url: string | null;
  channels: Partial<Record<CompositeChannel, string>>;
}

export const EMPTY_COMPOSITE_SYNC: CompositeSyncLedger = Object.freeze({ url: null, channels: Object.freeze({}) });

const CHANNELS: readonly CompositeChannel[] = ["r", "g", "b"];

export function wizardStepStaleAfterChannelSync(wizardCompositeReady: boolean): "stretch" | null {
  return wizardCompositeReady ? "stretch" : null;
}

export function advanceCompositeSync(ledger: CompositeSyncLedger, liveUrl: string | null, nextUrl: string): CompositeSyncLedger {
  const unchanged = ledger.url !== null && ledger.url === liveUrl;
  return { url: nextUrl, channels: unchanged ? ledger.channels : {} };
}

export function recordCompositeSync(
  ledger: CompositeSyncLedger,
  liveUrl: string | null,
  nextUrl: string,
  channel: CompositeChannel,
  fileKey: string,
): CompositeSyncLedger {
  const next = advanceCompositeSync(ledger, liveUrl, nextUrl);
  return { url: next.url, channels: { ...next.channels, [channel]: fileKey } };
}

export function syncedChannelFor(ledger: CompositeSyncLedger, liveUrl: string | null, fileKey: string): CompositeChannel | null {
  if (ledger.url === null || ledger.url !== liveUrl) return null;
  return CHANNELS.find((c) => ledger.channels[c] === fileKey) ?? null;
}

export function forgetCompositeSync(ledger: CompositeSyncLedger, channel: CompositeChannel): CompositeSyncLedger {
  if (ledger.channels[channel] === undefined) return ledger;
  const channels = { ...ledger.channels };
  delete channels[channel];
  return { url: ledger.url, channels };
}

let ledgerState: CompositeSyncLedger = EMPTY_COMPOSITE_SYNC;
let urlSeq = 0;

export const compositeSyncStore = {
  get: (): CompositeSyncLedger => ledgerState,
  set: (next: CompositeSyncLedger): void => {
    ledgerState = next;
  },
  tagUrl: (url: string): string => {
    urlSeq += 1;
    return withVersionParam(url, urlSeq);
  },
};
