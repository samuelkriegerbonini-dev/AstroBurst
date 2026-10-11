import { describe, it, expect, vi } from "vitest";
import { parentFolder, revealInFolder } from "../revealInFolder";

describe("revealInFolder", () => {
  it("reveals the saved file and reports nothing when the opener succeeds", async () => {
    const opener = vi.fn(async () => {});
    await expect(revealInFolder("C:/exports/m42_proc_20261010.fits", opener)).resolves.toBeNull();
    expect(opener).toHaveBeenCalledWith("C:/exports/m42_proc_20261010.fits");
  });

  it("names the folder and the reason when the opener throws an Error", async () => {
    const opener = vi.fn(async () => {
      throw new Error("No such file or directory");
    });
    await expect(revealInFolder("C:/exports/m42_proc_20261010.fits", opener)).resolves.toBe(
      "Could not open the folder C:/exports: No such file or directory",
    );
  });

  it("names the folder and the reason when the plugin rejects with a string", async () => {
    const opener = vi.fn(() => Promise.reject("org.freedesktop.portal.OpenURI is not available"));
    await expect(revealInFolder("/home/u/AstroBurst/exports/m42.png", opener)).resolves.toBe(
      "Could not open the folder /home/u/AstroBurst/exports: org.freedesktop.portal.OpenURI is not available",
    );
  });
});

describe("parentFolder", () => {
  it("drops the last path segment for either separator", () => {
    expect(parentFolder("C:\\Users\\me\\exports\\m42.fits")).toBe("C:\\Users\\me\\exports");
    expect(parentFolder("/data/out/aligned")).toBe("/data/out");
  });
});
