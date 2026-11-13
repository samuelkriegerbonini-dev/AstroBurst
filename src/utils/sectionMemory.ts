export type SectionMemoryTool = "processing" | "stacking";

const remembered = new Map<SectionMemoryTool, string>();

export const sectionMemory = {
  get<T extends string>(tool: SectionMemoryTool, allowed: readonly T[], fallback: T): T {
    const section = remembered.get(tool);
    return allowed.find((id) => id === section) ?? fallback;
  },
  set(tool: SectionMemoryTool, section: string): void {
    remembered.set(tool, section);
  },
  reset(): void {
    remembered.clear();
  },
};
