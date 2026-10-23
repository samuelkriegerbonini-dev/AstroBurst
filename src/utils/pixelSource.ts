interface ReloadedFile {
  reloads?: number;
}

export function pixelSourceVersion(file: ReloadedFile | null, processedPath: string | null, processedVersion: number): number {
  return processedPath ? processedVersion : file?.reloads ?? 0;
}
