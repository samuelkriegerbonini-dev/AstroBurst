import { useState, useCallback } from "react";
import JSZip from "jszip";
import { saveAs } from "file-saver";
import { FILE_STATUS } from "../utils/constants";
import { isTauri } from "../infrastructure/tauri";
import { fileKeyOf, getRenderRecord } from "../context/PreviewContext";
import { manifestLine, zipCandidates, zipEntryName } from "../utils/exportSources";
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
        throw new Error("ZIP export needs the desktop app: the preview PNGs live on disk.");
      }

      const { readFile } = await import("@tauri-apps/plugin-fs");
      const zip = new JSZip();
      const batchSize = 5;
      const missing: string[] = [];
      const manifest: string[] = [manifestLine("entry", "source", "file")];
      const taken = new Set<string>();
      let added = 0;

      for (let i = 0; i < doneFiles.length; i++) {
        const file = doneFiles[i];
        const key = fileKeyOf(file);
        const processed = key ? getRenderRecord(key)?.processed ?? null : null;
        const candidates = zipCandidates({ pngPath: file.result?.png_path ?? null, processed });

        let stored = false;
        for (const candidate of candidates) {
          try {
            const data = await readFile(candidate.path);
            const entry = zipEntryName(file.name, taken);
            zip.file(entry, data);
            manifest.push(manifestLine(entry, candidate.source, file.path));
            added++;
            stored = true;
            break;
          } catch (err) {
            console.error(`Failed to read ${candidate.path}:`, err);
          }
        }
        if (!stored) {
          missing.push(file.name);
          manifest.push(manifestLine(null, "skipped: no readable preview PNG", file.path));
        }

        setProgress(Math.round(((i + 1) / doneFiles.length) * 90));

        if ((i + 1) % batchSize === 0) {
          await new Promise((r) => requestAnimationFrame(() => setTimeout(r, 0)));
        }
      }

      if (added === 0) {
        throw new Error(`No preview image could be read for any of the ${doneFiles.length} files; nothing was downloaded.`);
      }

      zip.file("manifest.txt", manifest.join("\n") + "\n");

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
