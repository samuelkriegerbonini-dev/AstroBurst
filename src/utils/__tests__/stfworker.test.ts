import { afterAll, afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DisplayTransfer } from "../displayTransfer";

class FakeWorker {
  static last: FakeWorker | null = null;
  posted: Record<string, unknown>[] = [];
  onmessage: ((e: { data: Record<string, unknown> }) => void) | null = null;
  onerror: ((e: unknown) => void) | null = null;
  constructor() {
    FakeWorker.last = this;
  }
  postMessage(msg: Record<string, unknown>) {
    this.posted.push(msg);
  }
  terminate() {}
  reply(data: Record<string, unknown>) {
    this.onmessage?.({ data });
  }
}

vi.stubGlobal("Worker", FakeWorker);

const { renderStfInWorker, cancelPendingRenders, terminateStfWorker } = await import("../stfworker");

const TRANSFER = { stretch: "linear", vmin: 0, vmax: 1 } as unknown as DisplayTransfer;
const LUT = new Uint8Array(1024);

function track<T>(promise: Promise<T>) {
  const state: { settled: boolean; error: unknown } = { settled: false, error: null };
  promise.then(
    () => { state.settled = true; },
    (error: unknown) => { state.settled = true; state.error = error; },
  );
  return state;
}

async function flush(): Promise<void> {
  for (let i = 0; i < 5; i++) await Promise.resolve();
}

function lastRenderId(): number {
  const render = FakeWorker.last?.posted.filter((m) => m.type === "render").at(-1);
  return render?.id as number;
}

afterEach(() => {
  terminateStfWorker();
});

afterAll(() => {
  vi.unstubAllGlobals();
});

describe("renderStfInWorker", () => {
  it("rejects a render the worker reports it could not serve, so the CPU display does not freeze", async () => {
    const pending = track(renderStfInWorker({ transfer: TRANSFER, lut: LUT }));
    FakeWorker.last?.reply({ type: "failed", id: lastRenderId(), message: "no pixels" });
    await flush();
    expect(pending.settled).toBe(true);
    expect(String(pending.error)).toContain("no pixels");
  });

  it("rejects every pending render when the worker fails", async () => {
    const first = track(renderStfInWorker({ transfer: TRANSFER, lut: LUT }));
    const second = track(renderStfInWorker({ transfer: TRANSFER, lut: LUT }));
    FakeWorker.last?.onerror?.({ message: "worker crashed" });
    await flush();
    expect([first.settled, second.settled]).toEqual([true, true]);
  });

  it("rejects the renders it cancels instead of leaving them pending", async () => {
    const pending = track(renderStfInWorker({ transfer: TRANSFER, lut: LUT }));
    cancelPendingRenders();
    await flush();
    expect(pending.settled).toBe(true);
  });
});

describe("STF worker", () => {
  const posted: Record<string, unknown>[] = [];
  const scope: { onmessage: ((e: { data: Record<string, unknown> }) => void) | null; postMessage: (msg: Record<string, unknown>) => void } = {
    onmessage: null,
    postMessage: (msg) => posted.push(msg),
  };

  beforeEach(async () => {
    posted.length = 0;
    vi.stubGlobal("self", scope);
    vi.stubGlobal("ImageData", class { constructor(public data: Uint8ClampedArray, public width: number, public height: number) {} });
    vi.resetModules();
    await import("../stfworker.worker");
  });

  it("answers a render it has no pixels for with a failure", () => {
    scope.onmessage?.({ data: { type: "render", id: 7, transfer: TRANSFER, lut: LUT } });
    expect(posted).toEqual([{ type: "failed", id: 7, message: "the STF worker holds no pixels" }]);
  });

  it("answers a render whose bitmap cannot be made with a failure", async () => {
    vi.stubGlobal("createImageBitmap", () => Promise.reject(new Error("out of memory")));
    scope.onmessage?.({ data: { type: "render", id: 9, pixels: new Float32Array([0, 1]), width: 2, height: 1, transfer: TRANSFER, lut: LUT } });
    await flush();
    expect(posted).toEqual([{ type: "failed", id: 9, message: "out of memory" }]);
  });
});
