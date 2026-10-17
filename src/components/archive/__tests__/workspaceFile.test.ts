import { describe, expect, it } from "vitest";
import { footprintImage } from "../state/displayedImage";
import { workspaceFileRef } from "../state/useAroundTarget";
import { FILE_STATUS } from "../../../utils/constants";

const X1D_PATH = "C:/data/jw01266-o005_t001_nirspec_g395h-f290lp_x1d.fits";
const X1D_NAME = "jw01266-o005_t001_nirspec_g395h-f290lp_x1d.fits";
const IMAGE_PATH = "C:/data/673nmos.fits";

describe("the workspace file handed to the MAST archive", () => {
  it("is nothing for a 1D spectrum row, so neither the around search nor the footprints ask for its WCS", () => {
    const ref = workspaceFileRef(X1D_PATH, X1D_NAME, FILE_STATUS.SPECTRUM);
    expect(ref).toBeNull();
    expect(footprintImage(ref, { filePath: X1D_PATH, projectionPath: X1D_PATH })).toBeNull();
  });

  it("keeps a loaded image as the workspace file", () => {
    expect(workspaceFileRef(IMAGE_PATH, "673nmos.fits", FILE_STATUS.DONE)).toEqual({ path: IMAGE_PATH, name: "673nmos.fits" });
  });

  it("is nothing while no file is selected", () => {
    expect(workspaceFileRef(null, null, null)).toBeNull();
  });
});
