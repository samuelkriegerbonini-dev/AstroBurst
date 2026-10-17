export type DrizzleChannel = "r" | "g" | "b";

const AUTO_PATTERNS: [DrizzleChannel, RegExp][] = [
  ["r", /(?:[_.-](?:r|red)[_.-]|[_-](?:r|red)$|^(?:r|red)[_-])/i],
  ["g", /(?:[_.-](?:g|green)[_.-]|[_-](?:g|green)$|^(?:g|green)[_-])/i],
  ["b", /(?:[_.-](?:b|blue)[_.-]|[_-](?:b|blue)$|^(?:b|blue)[_-])/i],
];

export function autoDetectChannel(name: string): DrizzleChannel | null {
  const stem = name.replace(/\.(?:fits?|fts)(?:\.gz)?$/i, "");
  for (const [ch, re] of AUTO_PATTERNS) {
    if (re.test(stem)) return ch;
  }
  return null;
}
