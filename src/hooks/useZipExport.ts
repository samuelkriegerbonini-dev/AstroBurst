import { useState, useCallback } from "react";
import JSZip from "jszip";
import { saveAs } from "file-saver";
import { FILE_STATUS } from "../utils/constants";
import { isTauri } from "../infrastructure/tauri";
import type { ProcessedFile } from "../shared/types";

export function useZipExport() {
  const [progress, setProgress] = useState(0);
  const [isExporting, setIsExporting] = useState(false);
  const [downloaded, setDownloaded] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const exportZip = useCallback(async (files: ProcessedFile[]) => {
    const doneFiles = files.filter(
      (f) => f.status === FILE_STATUS.DONE && f.result,
    );
    if (doneFiles.length === 0) return;

    setIsExporting(true);
    setProgress(0);
    setError(null);

    try {
      if (!isTauri()) {
        throw new Error("ZIP export needs the desktop app: the processed PNGs live on disk.");
      }

      const zip = new JSZip();
      const batchSize = 5;
      const missing: string[] = [];
      let added = 0;

      for (let i = 0; i < doneFiles.length; i++) {
        const file = doneFiles[i];
        const pngName = file.name.replace(/\.fits?$/i, ".png");
        const pngPath = file.result?.png_path;

        if (pngPath) {
          try {
            const { readFile } = await import("@tauri-apps/plugin-fs");
            const data = await readFile(pngPath);
            zip.file(pngName, data);
            added++;
          } catch (err) {
            console.error(`Failed to read ${pngPath}:`, err);
            missing.push(file.name);
          }
        } else {
          missing.push(file.name);
        }

        setProgress(Math.round(((i + 1) / doneFiles.length) * 90));

        if ((i + 1) % batchSize === 0) {
          await new Promise((r) => requestAnimationFrame(() => setTimeout(r, 0)));
        }
      }

      if (added === 0) {
        throw new Error(`No processed image could be read for any of the ${doneFiles.length} files; nothing was downloaded.`);
      }

      const blob = await zip.generateAsync(
        {
          type: "blob",
          compression: "STORE",
        },
        (meta: { percent: number }) => {
          setProgress(90 + Math.round((meta.percent / 100) * 10));
        },
      );

      saveAs(blob, `astroburst-export-${Date.now()}.zip`);
      setDownloaded(true);
      setTimeout(() => setDownloaded(false), 2000);
      if (missing.length > 0) {
        setError(`${missing.length} of ${doneFiles.length} files were skipped: ${missing.slice(0, 3).join(", ")}${missing.length > 3 ? "..." : ""}`);
      }
    } catch (err) {
      console.error("ZIP export failed:", err);
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setIsExporting(false);
      setProgress(0);
    }
  }, []);

  return { exportZip, progress, isExporting, downloaded, error };
}
