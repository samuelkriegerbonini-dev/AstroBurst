import { describe, expect, it } from "vitest";
import { innermostLayer } from "../logic/escLayers";

const none = { menuOpen: false, largeOpen: false, drawerOpen: false, tokenChecking: false, searching: false };

describe("innermostLayer", () => {
  it("closes the menu first", () => {
    expect(innermostLayer({ menuOpen: true, largeOpen: true, drawerOpen: true, tokenChecking: true, searching: true })).toBe("menu");
  });

  it("then the Large view, then the drawer", () => {
    expect(innermostLayer({ ...none, largeOpen: true, drawerOpen: true, searching: true })).toBe("large");
    expect(innermostLayer({ ...none, drawerOpen: true, searching: true })).toBe("drawer");
  });

  it("then a token check, then a running search", () => {
    expect(innermostLayer({ ...none, tokenChecking: true, searching: true })).toBe("token");
    expect(innermostLayer({ ...none, searching: true })).toBe("search");
  });

  it("returns null when nothing is open", () => {
    expect(innermostLayer(none)).toBeNull();
  });
});
