import { describe, it, expect, expectTypeOf, beforeEach, vi } from "vitest";

const { typedInvokeMock } = vi.hoisted(() => ({ typedInvokeMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({ typedInvoke: typedInvokeMock }));

import { cleanupHistory, cleanupOutput, cleanupStorage, summarizeCleanup, type AppConfig } from "../config";

describe("AppConfig", () => {
  it("declares exactly the settings Rust AppConfig serializes, so no control writes a field update_config no longer has", () => {
    expectTypeOf<keyof AppConfig>().toEqualTypeOf<
      | "astrometry_api_key"
      | "astrometry_api_url"
      | "plate_solve_timeout_secs"
      | "output_max_size_mb"
      | "mast_token_transport"
      | "data_dir_override"
      | "mast_quota_gb"
    >();
    expectTypeOf<AppConfig["data_dir_override"]>().toEqualTypeOf<string | null>();
    expectTypeOf<AppConfig["mast_quota_gb"]>().toEqualTypeOf<number | null>();
  });
});

describe("cleanupOutput", () => {
  beforeEach(() => {
    typedInvokeMock.mockReset();
    typedInvokeMock.mockResolvedValue({ cleaned_files: 0, cleaned_bytes: 0, cleaned_paths: [], total_size: 0, file_count: 0, output_dir: "/out", elapsed_ms: 0 });
  });

  it("sends the paths the frontend still uses so the sweep never deletes them", async () => {
    await cleanupOutput("/out", ["/out/a.png", "/out/a_resampled.fits"]);
    expect(typedInvokeMock).toHaveBeenCalledWith("cleanup_output_cmd", {
      outputDir: "/out",
      maxSizeMb: null,
      keep: ["/out/a.png", "/out/a_resampled.fits"],
    });
  });
});

describe("cleanupHistory", () => {
  beforeEach(() => {
    typedInvokeMock.mockReset();
    typedInvokeMock.mockResolvedValue({ cleaned_files: 0, cleaned_bytes: 0, cleaned_paths: [], replayable_nodes: 0, total_size: 0, file_count: 0, history_root: "/app/history", elapsed_ms: 0 });
  });

  it("sends only the history root, so Rust's pinned set is the single guard", async () => {
    await cleanupHistory("/app/history");
    expect(typedInvokeMock).toHaveBeenCalledWith("cleanup_history_cmd", { historyRoot: "/app/history", maxSizeMb: null });
  });
});

describe("cleanupStorage", () => {
  beforeEach(() => {
    typedInvokeMock.mockReset();
    typedInvokeMock.mockImplementation(async (cmd: string) =>
      cmd === "cleanup_output_cmd"
        ? { cleaned_files: 2, cleaned_bytes: 10, cleaned_paths: ["/app/output/a.fits"], total_size: 5, file_count: 1, output_dir: "/app/output", elapsed_ms: 1 }
        : { cleaned_files: 3, cleaned_bytes: 20, cleaned_paths: ["/app/history/id/n0002.fits"], replayable_nodes: 1, total_size: 7, file_count: 4, history_root: "/app/history", elapsed_ms: 1 },
    );
  });

  it("sweeps the output folder with the keep list and the history root without it", async () => {
    const res = await cleanupStorage("/app/output", "/app/history", ["/app/output/m42.png"]);
    expect(typedInvokeMock.mock.calls).toEqual([
      ["cleanup_output_cmd", { outputDir: "/app/output", maxSizeMb: null, keep: ["/app/output/m42.png"] }],
      ["cleanup_history_cmd", { historyRoot: "/app/history", maxSizeMb: null }],
    ]);
    expect(res.output.ok && res.output.result.total_size).toBe(5);
    expect(res.history.ok && res.history.result.replayable_nodes).toBe(1);
    expect(res.history.ok && res.history.result.total_size).toBe(7);
  });

  it("still reports the output sweep's deleted files when the history sweep fails", async () => {
    typedInvokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "cleanup_history_cmd") throw new Error("refusing to sweep ./history");
      return { cleaned_files: 1, cleaned_bytes: 10, cleaned_paths: ["/app/output/a.fits"], total_size: 5, file_count: 1, output_dir: "/app/output", elapsed_ms: 1 };
    });
    const res = await cleanupStorage("/app/output", "./history", []);
    expect(res.output.ok && res.output.result.cleaned_paths).toEqual(["/app/output/a.fits"]);
    expect(res.history).toEqual({ ok: false, error: "refusing to sweep ./history" });
    expect(summarizeCleanup(res)).toEqual({
      cleanedPaths: ["/app/output/a.fits"],
      cleanedFiles: 1,
      cleanedBytes: 10,
      replayableNodes: 0,
      errors: ["History cleanup failed: refusing to sweep ./history"],
    });
  });

  it("still sweeps history when the output sweep fails", async () => {
    typedInvokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "cleanup_output_cmd") throw "Failed to read output directory";
      return { cleaned_files: 3, cleaned_bytes: 20, cleaned_paths: [], replayable_nodes: 2, total_size: 7, file_count: 4, history_root: "/app/history", elapsed_ms: 1 };
    });
    const res = await cleanupStorage("/app/output", "/app/history", []);
    expect(res.output).toEqual({ ok: false, error: "Failed to read output directory" });
    expect(res.history.ok && res.history.result.replayable_nodes).toBe(2);
    const summary = summarizeCleanup(res);
    expect([summary.cleanedFiles, summary.replayableNodes]).toEqual([3, 2]);
    expect(summary.errors).toEqual(["Output cleanup failed: Failed to read output directory"]);
  });
});
