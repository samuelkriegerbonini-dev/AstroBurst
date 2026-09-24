import { describe, it, expect } from "vitest";
import {
  EMPTY_COMPOSITE_SYNC,
  advanceCompositeSync,
  compositeSyncStore,
  forgetCompositeSync,
  recordCompositeSync,
  syncedChannelFor,
  wizardStepStaleAfterChannelSync,
} from "../compositeSync";
import { INITIAL_STATE, invalidateDownstream, type WizardState } from "../wizard";

const WIZARD_URL = "asset://out/composite.png";

describe("wizard state after a composite channel sync", () => {
  it("reopens Stretch, Adjust and Export of a blended wizard, whose stretched and toned tiers the sync dropped", () => {
    const done: WizardState = {
      ...INITIAL_STATE,
      compositeReady: true,
      completedSteps: { channels: true, blend: true, colorbalance: true, stretch: true, adjust: true, export: true },
    };
    const step = wizardStepStaleAfterChannelSync(done.compositeReady);
    expect(step).toBe("stretch");
    const after = { ...done, ...invalidateDownstream(done, step!) };
    expect(after.completedSteps).toEqual({ channels: true, blend: true, colorbalance: true });
    expect(after.compositeReady).toBe(true);
  });

  it("leaves a wizard without a blended composite alone", () => {
    expect(wizardStepStaleAfterChannelSync(false)).toBeNull();
  });
});

describe("composite sync ledger", () => {
  it("a file whose step never reached the composite has no channel to restore", () => {
    expect(syncedChannelFor(EMPTY_COMPOSITE_SYNC, WIZARD_URL, "1|/data/Ha.fits")).toBeNull();
  });

  it("a sync is restorable while the composite still shows the synced render", () => {
    const ledger = recordCompositeSync(EMPTY_COMPOSITE_SYNC, WIZARD_URL, "u?v=1", "r", "1|/data/Ha.fits");
    expect(syncedChannelFor(ledger, "u?v=1", "1|/data/Ha.fits")).toBe("r");
    expect(syncedChannelFor(ledger, "u?v=1", "2|/data/OIII.fits")).toBeNull();
  });

  it("any composite change made by someone else invalidates the sync", () => {
    const ledger = recordCompositeSync(EMPTY_COMPOSITE_SYNC, WIZARD_URL, "u?v=1", "r", "1|/data/Ha.fits");
    expect(syncedChannelFor(ledger, WIZARD_URL, "1|/data/Ha.fits")).toBeNull();
    expect(syncedChannelFor(ledger, null, "1|/data/Ha.fits")).toBeNull();
  });

  it("consecutive syncs from different files keep each other's channels", () => {
    let ledger = recordCompositeSync(EMPTY_COMPOSITE_SYNC, WIZARD_URL, "u?v=1", "r", "1|/data/Ha.fits");
    ledger = recordCompositeSync(ledger, "u?v=1", "u?v=2", "g", "2|/data/OIII.fits");
    expect(syncedChannelFor(ledger, "u?v=2", "1|/data/Ha.fits")).toBe("r");
    expect(syncedChannelFor(ledger, "u?v=2", "2|/data/OIII.fits")).toBe("g");
  });

  it("a sync after a foreign change starts a fresh ledger", () => {
    let ledger = recordCompositeSync(EMPTY_COMPOSITE_SYNC, WIZARD_URL, "u?v=1", "r", "1|/data/Ha.fits");
    ledger = recordCompositeSync(ledger, WIZARD_URL, "u?v=2", "g", "2|/data/OIII.fits");
    expect(syncedChannelFor(ledger, "u?v=2", "1|/data/Ha.fits")).toBeNull();
    expect(syncedChannelFor(ledger, "u?v=2", "2|/data/OIII.fits")).toBe("g");
  });

  it("restoring one channel keeps the other synced channels restorable", () => {
    let ledger = recordCompositeSync(EMPTY_COMPOSITE_SYNC, WIZARD_URL, "u?v=1", "r", "1|/data/Ha.fits");
    ledger = recordCompositeSync(ledger, "u?v=1", "u?v=2", "g", "2|/data/OIII.fits");
    ledger = forgetCompositeSync(ledger, "r");
    expect(syncedChannelFor(ledger, "u?v=2", "1|/data/Ha.fits")).toBeNull();
    ledger = advanceCompositeSync(ledger, "u?v=2", "u?v=3");
    expect(syncedChannelFor(ledger, "u?v=3", "2|/data/OIII.fits")).toBe("g");
  });

  it("tags every render with a distinct URL even when the PNG path repeats", () => {
    const a = compositeSyncStore.tagUrl(WIZARD_URL);
    const b = compositeSyncStore.tagUrl(WIZARD_URL);
    expect(a).not.toBe(b);
    expect(a).not.toBe(WIZARD_URL);
  });
});
