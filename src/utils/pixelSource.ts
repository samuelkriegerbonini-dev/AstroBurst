interface ReloadedFile {
  reloads?: number;
}

interface ScientificInput extends ReloadedFile {
  id: string;
  path: string;
  imageRef?: string | null;
  finishedAt?: number | null;
}

export function scientificInputRevisionOf(file: ScientificInput | null): string {
  return JSON.stringify([file ? `${file.id}|${file.path}` : null, file?.imageRef, file?.reloads, file?.finishedAt]);
}

export function pixelSourceVersion(file: ReloadedFile | null, processedPath: string | null, processedVersion: number): number {
  return processedPath ? processedVersion : file?.reloads ?? 0;
}
