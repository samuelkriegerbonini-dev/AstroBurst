import { describe, expect, it } from "vitest";
import { OUTPUT_FOLDER_WARNING, SHOW_IN_FOLDER, outputNote } from "../outputNote";

const WINDOWS_M0 = "C:\\Users\\me\\AppData\\Roaming\\com.astroburst.desktop\\output\\jw01266_s3d_moment0_5-39.fits";
const POSIX_PV = "/home/me/.local/share/com.astroburst.desktop/output/cube_pv_r1.fits";

describe("outputNote", () => {
  it("names the written file and warns that the output folder is cleared when the app starts", () => {
    const note = outputNote(WINDOWS_M0);
    expect(note.name).toBe("jw01266_s3d_moment0_5-39.fits");
    expect(note.warning).toBe("In the output folder, which is cleared when AstroBurst starts: Export > FITS keeps a copy of the image on screen.");
    expect(note.warning).toBe(OUTPUT_FOLDER_WARNING);
  });

  it("keeps the full path for Show in folder with either separator", () => {
    expect(outputNote(WINDOWS_M0).path).toBe(WINDOWS_M0);
    expect(outputNote(POSIX_PV)).toEqual({ path: POSIX_PV, name: "cube_pv_r1.fits", warning: OUTPUT_FOLDER_WARNING });
  });

  it("labels the reveal action Show in folder", () => {
    expect(SHOW_IN_FOLDER).toBe("Show in folder");
  });
});
