import { useState, useCallback } from "react";
import JSZip from "jszip";
import { saveAs } from "file-saver";
import { FILE_STATUS } from "../utils/constants";
import type { ProcessedFile } from "../utils/types";

export function useZipExport() {
  const [progress, setProgress] = useState(0);
  const [isExporting, setIsExporting] = useState(false);
  const [downloaded, setDownloaded] = useState(false);

  const exportZip = useCallback(async (files: ProcessedFile[]) => {
    const doneFiles = files.filter(
      (f) => f.status === FILE_STATUS.DONE && f.result,
    );
    if (doneFiles.length === 0) return;

    setIsExporting(true);
    setProgress(0);

    try {
      const zip = new JSZip();

      for (let i = 0; i < doneFiles.length; i++) {
        const file = doneFiles[i];
        const pngName = file.name.replace(/\.fits?$/i, ".png");

        if ((window as any).__TAURI_INTERNALS__ && file.result?.png_path) {
          try {
            const { readFile } = await import("@tauri-apps/plugin-fs");
            const data = await readFile(file.result.png_path);
            zip.file(pngName, data);
          } catch (err) {
            console.error(`Failed to read ${file.result.png_path}:`, err);
          }
        }

        setProgress(Math.round(((i + 1) / doneFiles.length) * 100));
      }

      const blob = await zip.generateAsync({
        type: "blob",
        compression: "DEFLATE",
        compressionOptions: { level: 6 },
      });

      saveAs(blob, `astrokit-export-${Date.now()}.zip`);
      setDownloaded(true);
      setTimeout(() => setDownloaded(false), 2000);
    } catch (err) {
      console.error("ZIP export failed:", err);
    } finally {
      setIsExporting(false);
      setProgress(0);
    }
  }, []);

  return { exportZip, progress, isExporting, downloaded };
}
