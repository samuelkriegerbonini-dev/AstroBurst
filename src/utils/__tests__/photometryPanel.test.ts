import { describe, expect, it } from "vitest";
import type { PhotometryMeasurement } from "../../services/analysis";
import {
  GAIA_MATCH_PREF_KEY,
  createClickLogGate,
  detectedStarsLabel,
  gaiaFailureWarning,
  loadGaiaMatchPreference,
  replaceNewest,
  saveGaiaMatchPreference,
  withWarning,
} from "../photometryPanel";

function memoryStorage(initial: Record<string, string> = {}) {
  const map = new Map(Object.entries(initial));
  return {
    getItem: (k: string) => map.get(k) ?? null,
    setItem: (k: string, v: string) => {
      map.set(k, v);
    },
    map,
  };
}

function measurement(warnings: string[] = []): PhotometryMeasurement {
  return {
    photometry: {} as PhotometryMeasurement["photometry"],
    sky: null,
    gaia: null,
    photcal: null,
    warnings,
    masked: false,
    elapsed_ms: 1,
  };
}

describe("Gaia match preference", () => {
  it("is off when nothing is stored", () => {
    expect(loadGaiaMatchPreference(memoryStorage())).toBe(false);
  });

  it("remembers the last choice", () => {
    const s = memoryStorage();
    saveGaiaMatchPreference(true, s);
    expect(s.map.get(GAIA_MATCH_PREF_KEY)).toBe("true");
    expect(loadGaiaMatchPreference(s)).toBe(true);
    saveGaiaMatchPreference(false, s);
    expect(loadGaiaMatchPreference(s)).toBe(false);
  });

  it("treats a garbage value as off", () => {
    expect(loadGaiaMatchPreference(memoryStorage({ [GAIA_MATCH_PREF_KEY]: "yes" }))).toBe(false);
  });

  it("is off when the storage throws, and saving into a throwing storage does not throw", () => {
    const throwing = {
      getItem: (): string | null => {
        throw new Error("blocked");
      },
      setItem: (): void => {
        throw new Error("blocked");
      },
    };
    expect(loadGaiaMatchPreference(throwing)).toBe(false);
    expect(() => saveGaiaMatchPreference(true, throwing)).not.toThrow();
  });
});

describe("Gaia failure warning", () => {
  it("names the reason of an Error and of a plain string", () => {
    expect(gaiaFailureWarning(new Error("timed out"))).toBe("Gaia query failed: timed out");
    expect(gaiaFailureWarning("offline")).toBe("Gaia query failed: offline");
  });

  it("adds a warning once without touching the original result", () => {
    const base = measurement(["saturated core"]);
    const once = withWarning(base, "Gaia query failed: offline");
    expect(once.warnings).toEqual(["saturated core", "Gaia query failed: offline"]);
    expect(base.warnings).toEqual(["saturated core"]);
    expect(withWarning(once, "Gaia query failed: offline")).toBe(once);
  });
});

describe("replaceNewest", () => {
  it("swaps the newest entry when it is the local measurement", () => {
    const local = measurement();
    const older = measurement();
    const matched = measurement(["Gaia: no G<17 star within 5\""]);
    expect(replaceNewest([local, older], local, matched)).toEqual([matched, older]);
  });

  it("leaves the history alone when a newer click already replaced the head", () => {
    const local = measurement();
    const newer = measurement();
    const history = [newer, local];
    expect(replaceNewest(history, local, measurement())).toBe(history);
    const empty: PhotometryMeasurement[] = [];
    expect(replaceNewest(empty, local, measurement())).toBe(empty);
  });
});

describe("click log gate", () => {
  function recorder() {
    const rows: string[] = [];
    return { rows, append: (row: string) => () => void rows.push(row) };
  }

  it("logs one row for a click whose panel goes away while Gaia is pending", () => {
    const gate = createClickLogGate();
    const log = recorder();
    const seq = gate.begin();
    gate.hold(seq, log.append("local"));
    gate.supersede();
    expect(gate.isCurrent(seq)).toBe(false);
    gate.commit(seq, log.append("matched"));
    expect(log.rows).toEqual(["local"]);
  });

  it("logs only the matched row when Gaia answers before anything else happens", () => {
    const gate = createClickLogGate();
    const log = recorder();
    const seq = gate.begin();
    gate.hold(seq, log.append("local"));
    expect(gate.isCurrent(seq)).toBe(true);
    gate.commit(seq, log.append("matched"));
    gate.supersede();
    gate.begin();
    expect(log.rows).toEqual(["matched"]);
  });

  it("logs the held row of the previous click when a new click starts, and drops its late match", () => {
    const gate = createClickLogGate();
    const log = recorder();
    const first = gate.begin();
    gate.hold(first, log.append("A local"));
    const second = gate.begin();
    expect(log.rows).toEqual(["A local"]);
    gate.commit(first, log.append("A matched"));
    gate.commit(second, log.append("B local"));
    expect(log.rows).toEqual(["A local", "B local"]);
  });

  it("ignores a hold that arrives after the click was superseded", () => {
    const gate = createClickLogGate();
    const log = recorder();
    const seq = gate.begin();
    gate.supersede();
    gate.hold(seq, log.append("stale"));
    gate.supersede();
    gate.begin();
    expect(log.rows).toEqual([]);
  });
});

describe("detectedStarsLabel", () => {
  it("names the cap when detection found more stars than it kept", () => {
    expect(detectedStarsLabel(200, 3120)).toBe("Detected stars (200 of 3120)");
  });

  it("shows the plain count when nothing was dropped or the total is unknown", () => {
    expect(detectedStarsLabel(57, 57)).toBe("Detected stars (57)");
    expect(detectedStarsLabel(57, null)).toBe("Detected stars (57)");
    expect(detectedStarsLabel(57, undefined)).toBe("Detected stars (57)");
    expect(detectedStarsLabel(0, 3120)).toBe("Detected stars (0)");
    expect(detectedStarsLabel(200, Number.NaN)).toBe("Detected stars (200)");
  });
});
