import { describe, expect, it } from "vitest";
import { placeHelpCard } from "../helpCardPlacement";

const VIEWPORT = { width: 1280, height: 800 };

describe("placeHelpCard", () => {
  it("opens below the button when the card fits", () => {
    expect(placeHelpCard({ left: 300, top: 40, bottom: 62 }, 120, VIEWPORT)).toEqual({ left: 292, top: 70, width: 280 });
  });

  it("flips above the button when there is no room below", () => {
    expect(placeHelpCard({ left: 300, top: 700, bottom: 722 }, 120, VIEWPORT)).toEqual({ left: 292, top: 572, width: 280 });
  });

  it("opens below when the card ends exactly 8 px above the bottom edge", () => {
    expect(placeHelpCard({ left: 300, top: 650, bottom: 672 }, 112, VIEWPORT).top).toBe(680);
  });

  it("keeps a flipped card 8 px from the top edge", () => {
    expect(placeHelpCard({ left: 300, top: 50, bottom: 72 }, 760, VIEWPORT).top).toBe(8);
  });

  it("narrows the card to the viewport and clamps it to the left edge", () => {
    expect(placeHelpCard({ left: 150, top: 40, bottom: 62 }, 120, { width: 240, height: 800 })).toEqual({ left: 8, top: 70, width: 224 });
  });

  it("keeps the card 8 px inside the right and left edges", () => {
    expect(placeHelpCard({ left: 1200, top: 40, bottom: 62 }, 120, VIEWPORT).left).toBe(992);
    expect(placeHelpCard({ left: 4, top: 40, bottom: 62 }, 120, VIEWPORT).left).toBe(8);
  });
});
