import { createContext, useContext } from "react";

export const ToolHostContext = createContext<{ active: boolean; gpuDisplay: boolean }>({ active: true, gpuDisplay: true });

export function useToolHost(): { active: boolean; gpuDisplay: boolean } {
  return useContext(ToolHostContext);
}
