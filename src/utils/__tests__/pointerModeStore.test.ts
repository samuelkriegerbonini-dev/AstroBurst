import { afterEach, describe, expect, it, vi } from "vitest";
import { hooks } from "../../test/hookRuntime";
import { pointerModeStore, usePointerMode } from "../pointerModeStore";

vi.mock("react", async (original) => (await import("../../test/hookRuntime")).reactWithFakeHooks(await original()));

function PointerModeReader(): string {
  return usePointerMode();
}

describe("pointer mode store", () => {
  afterEach(() => {
    hooks.mount();
    pointerModeStore.set("pan");
  });

  it("starts in Pan", () => {
    expect(pointerModeStore.get()).toBe("pan");
    expect(hooks.render(PointerModeReader, {})).toBe("pan");
  });

  it("gives Crosshair to a viewer mounted after another viewer picked it", () => {
    hooks.render(PointerModeReader, {});
    pointerModeStore.set("crosshair");
    hooks.mount();
    expect(hooks.render(PointerModeReader, {})).toBe("crosshair");
  });

  it("tells subscribers once per change of mode", () => {
    const listener = vi.fn();
    const unsubscribe = pointerModeStore.subscribe(listener);
    pointerModeStore.set("crosshair");
    pointerModeStore.set("crosshair");
    unsubscribe();
    pointerModeStore.set("pan");
    expect(listener).toHaveBeenCalledTimes(1);
  });
});
