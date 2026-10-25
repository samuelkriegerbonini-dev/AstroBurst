import { fileBaseName } from "./values";

export const OUTPUT_FOLDER_WARNING = "In the output folder, which is cleared when AstroBurst starts: Export > FITS keeps a copy of the image on screen.";

export const SHOW_IN_FOLDER = "Show in folder";

export interface OutputNote {
  path: string;
  name: string;
  warning: string;
}

export function outputNote(path: string): OutputNote {
  return { path, name: fileBaseName(path) || path, warning: OUTPUT_FOLDER_WARNING };
}
