import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SaguiConfig } from "../../shared/types/sagui";

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }));
vi.mock("../../infrastructure/tauri", () => ({ typedInvoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));
import { cancelSagui, cancelSaguiInstall, inspectSaguiInput, inspectSaguiRuntime, installSaguiRuntime, loadSaguiResult, onSaguiInstallProgress, onSaguiProgress, runSagui } from "../sagui";

beforeEach(() => vi.clearAllMocks());

describe("SAGUI desktop command contract", () => {
  it("loads a previous manifest through backend integrity checks without running R", async () => {
    const saved = { result: { run_id: "saved" }, config: { input_path: "C:\\data\\cube.fits" } };
    mocks.invoke.mockResolvedValueOnce(saved);
    expect(await loadSaguiResult("C:\\old runs\\manifest.json")).toEqual(saved);
    expect(mocks.invoke).toHaveBeenCalledWith("load_sagui_result_cmd", { manifestPath: "C:\\old runs\\manifest.json" });
  });
  it("installs with argument-safe paths and a separate progress/cancellation event", async () => {
    await installSaguiRuntime("C:\\Program Files\\R\\Rscript.exe", "C:\\private library", "install-1");
    const progress = vi.fn();
    await onSaguiInstallProgress("install-1", progress);
    const [event, listener] = mocks.listen.mock.calls[0];
    listener({ payload: { current: 1, total: 4, percent: 25, stage: "dependencies" } });
    await cancelSaguiInstall("install-1");
    expect(mocks.invoke).toHaveBeenCalledWith("install_sagui_runtime_cmd", { rscriptPath: "C:\\Program Files\\R\\Rscript.exe", outputDir: "C:\\private library", runId: "install-1" });
    expect(event).toBe("sagui-install-progress-install-1");
    expect(progress).toHaveBeenCalledOnce();
    expect(mocks.invoke).toHaveBeenCalledWith("cancel_progress_cmd", { event });
  });

  it("passes runtime paths and config without invoking a shell", async () => {
    const config = { input_path: "C:\\cube.fits", mode: "ward" } as SaguiConfig;
    await inspectSaguiInput(config.input_path);
    await inspectSaguiRuntime("C:\\Program Files\\R\\Rscript.exe", "C:\\R library");
    await runSagui(config, "job-1");
    expect(mocks.invoke.mock.calls).toEqual([
      ["inspect_sagui_input_cmd", { path: config.input_path }],
      ["inspect_sagui_runtime_cmd", { rscriptPath: "C:\\Program Files\\R\\Rscript.exe", libraryPath: "C:\\R library" }],
      ["run_sagui_cmd", { config, runId: "job-1" }],
    ]);
  });

  it("cancels only the event of the current SAGUI run", async () => {
    const receive = vi.fn();
    await onSaguiProgress("job-2", receive);
    const [event, listener] = mocks.listen.mock.calls[0];
    const payload = { current: 3, total: 5, percent: 60, stage: "segment" };
    listener({ payload });
    await cancelSagui("job-2");
    expect(event).toBe("sagui-progress-job-2");
    expect(receive).toHaveBeenCalledWith(payload);
    expect(mocks.invoke).toHaveBeenCalledWith("cancel_progress_cmd", { event });
  });
});
