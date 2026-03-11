import { useState, useEffect } from "react";
import { getOutputDir } from "../utils/outputdir";

export function useOutputDir(): string | null {
  const [dir, setDir] = useState<string | null>(null);

  useEffect(() => {
    getOutputDir().then(setDir);
  }, []);

  return dir;
}
