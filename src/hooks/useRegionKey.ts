import { useFileContext } from "../context/PreviewContext";

export function useRegionKey(): string | null {
  const { file } = useFileContext();
  return file?.path ?? null;
}
