import { describe, it, expect } from "vitest";
import { quietCompletion } from "../quietCompletion";

describe("quietCompletion", () => {
  it("keeps the completion quiet for a quiet add after the batch finished", () => {
    expect(quietCompletion(true, true, false)).toBe(true);
  });

  it("lets a user batch still in progress celebrate when a quiet add joins it", () => {
    expect(quietCompletion(true, false, false)).toBe(false);
  });

  it("lets files the user adds during a quiet add celebrate", () => {
    expect(quietCompletion(false, false, true)).toBe(false);
  });

  it("stays quiet when a second quiet add arrives while the first is still processing", () => {
    expect(quietCompletion(true, false, true)).toBe(true);
  });
});
