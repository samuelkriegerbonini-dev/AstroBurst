import { describe, expect, it } from "vitest";
import { ARMED_CONFIRM_WINDOW_MS, DISARMED, armedConfirmStep, type ArmedState } from "../useArmedConfirm";

const T0 = 1_000_000;

function press(state: ArmedState, key: string, now: number) {
  return armedConfirmStep(state, { type: "press", key }, now);
}

describe("armedConfirmStep", () => {
  it("arms on the first press without running the action", () => {
    const step = press(DISARMED, "clear:R", T0);
    expect(step.run).toBe(false);
    expect(step.state).toEqual({ armedKey: "clear:R", armedAt: T0 });
  });

  it("runs the action on a second press of the same key inside the window and disarms", () => {
    const armed = press(DISARMED, "clear:R", T0).state;
    const step = press(armed, "clear:R", T0 + ARMED_CONFIRM_WINDOW_MS - 1);
    expect(step.run).toBe(true);
    expect(step.state.armedKey).toBeNull();
  });

  it("re-arms instead of running when the second press comes after the window", () => {
    const armed = press(DISARMED, "clear:R", T0).state;
    const late = T0 + ARMED_CONFIRM_WINDOW_MS + 500;
    const step = press(armed, "clear:R", late);
    expect(step.run).toBe(false);
    expect(step.state).toEqual({ armedKey: "clear:R", armedAt: late });
  });

  it("re-arms on another key instead of running the first one", () => {
    const armed = press(DISARMED, "clear:R", T0).state;
    const step = press(armed, "clear:G", T0 + 100);
    expect(step.run).toBe(false);
    expect(step.state).toEqual({ armedKey: "clear:G", armedAt: T0 + 100 });
  });

  it("disarms on the timeout scheduled for the current arm", () => {
    const armed = press(DISARMED, "delete:a", T0).state;
    const timeout = { type: "timeout", key: "delete:a", armedAt: T0 } as const;
    expect(armedConfirmStep(armed, timeout, T0 + ARMED_CONFIRM_WINDOW_MS).state).toEqual(DISARMED);
  });

  it("disarms on its own timeout even when the wall clock reads inside the window", () => {
    const armed = press(DISARMED, "clear:R", T0).state;
    const early = armedConfirmStep(armed, { type: "timeout", key: "clear:R", armedAt: T0 }, T0 + 10);
    expect(early.run).toBe(false);
    expect(early.state).toEqual(DISARMED);
  });

  it("ignores a timeout scheduled for an earlier arm", () => {
    const first = press(DISARMED, "delete:a", T0).state;
    const other = press(first, "delete:b", T0 + 100).state;
    expect(armedConfirmStep(other, { type: "timeout", key: "delete:a", armedAt: T0 }, T0 + 4000).state).toEqual(other);
    const late = T0 + ARMED_CONFIRM_WINDOW_MS + 500;
    const rearmed = press(first, "delete:a", late).state;
    expect(armedConfirmStep(rearmed, { type: "timeout", key: "delete:a", armedAt: T0 }, late + 1).state).toEqual(rearmed);
  });

  it("disarms when the armed button loses focus and ignores blur of another button", () => {
    const armed = press(DISARMED, "delete:a", T0).state;
    expect(armedConfirmStep(armed, { type: "blur", key: "delete:a" }, T0 + 10).state.armedKey).toBeNull();
    expect(armedConfirmStep(armed, { type: "blur", key: "delete:b" }, T0 + 10).state).toEqual(armed);
    const afterBlur = armedConfirmStep(armed, { type: "blur", key: "delete:a" }, T0 + 10).state;
    expect(press(afterBlur, "delete:a", T0 + 20).run).toBe(false);
  });
});
