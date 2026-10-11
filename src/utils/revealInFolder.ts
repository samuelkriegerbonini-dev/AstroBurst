export type RevealOpener = (path: string) => Promise<void>;

async function defaultOpener(path: string): Promise<void> {
  const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
  await revealItemInDir(path);
}

export function parentFolder(path: string): string {
  return path.replace(/[/\\][^/\\]+$/, "");
}

export async function revealInFolder(path: string, opener: RevealOpener = defaultOpener): Promise<string | null> {
  try {
    await opener(path);
    return null;
  } catch (e) {
    const reason = e instanceof Error ? e.message : String(e);
    return `Could not open the folder ${parentFolder(path)}: ${reason}`;
  }
}
