import { describe, it, expect, expectTypeOf, beforeEach, vi } from "vitest";

const { typedInvokeMock } = vi.hoisted(() => ({ typedInvokeMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({ typedInvoke: typedInvokeMock }));

import { cleanupOutput, type AppConfig } from "../config";

describe("AppConfig", () => {
  it("declares exactly the settings Rust AppConfig serializes, so no control writes a field update_config no longer has", () => {
    expectTypeOf<keyof AppConfig>().toEqualTypeOf<
      "astrometry_api_key" | "astrometry_api_url" | "plate_solve_timeout_secs" | "output_max_size_mb"
    >();
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
