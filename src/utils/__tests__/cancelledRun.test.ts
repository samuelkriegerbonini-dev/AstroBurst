import { describe, expect, it } from "vitest";
import { isCancelledRun } from "../cancelledRun";
import { isCancelledRun as drizzleCancelled } from "../drizzleRgbResult";
import { isCancelMessage } from "../../hooks/useProcessingRun";

describe("isCancelledRun", () => {
  it("recognises the backend cancel error, alone or at the end of its context chain", () => {
    for (const message of ["Operation cancelled", "dark: Operation cancelled", "Stacking failed: Operation cancelled", "Task join failed: CANCELLED"]) {
      expect(isCancelledRun(message), message).toBe(true);
    }
  });

  it("does not read an ordinary error that mentions a cancel as a user cancel", () => {
    for (const message of ["Failed to read frame_3.fits: read cancelled by the system", "Upload cancelled by server: quota", "File not found", ""]) {
      expect(isCancelledRun(message), message).toBe(false);
    }
  });

  it("is the one test every stacking and processing run uses", () => {
    const message = "Failed to read frame_3.fits: read cancelled by the system";
    expect(drizzleCancelled(message)).toBe(false);
    expect(isCancelMessage(message)).toBe(false);
  });
});
