export const MAST_TOKEN_PAGE_URL = "https://auth.mast.stsci.edu/token?suggested_name=AstroBurst&suggested_scope=mast:exclusive_access";
export const TOKEN_MIN_LENGTH = 16;
export const TOKEN_MAX_LENGTH = 512;

export type TokenFormat = { ok: true; token: string } | { ok: false; reason: string };

function printableAscii(text: string): boolean {
  for (let i = 0; i < text.length; i += 1) {
    const code = text.charCodeAt(i);
    if (code < 0x21 || code > 0x7e) return false;
  }
  return true;
}

export function checkTokenFormat(raw: string): TokenFormat {
  const token = raw.trim();
  if (!token) return { ok: false, reason: "Paste the token you created on the MAST token page." };
  if (!printableAscii(token)) return { ok: false, reason: "A MAST token has only printable ASCII characters and no spaces." };
  if (token.length < TOKEN_MIN_LENGTH) return { ok: false, reason: `This is too short for a MAST token (at least ${TOKEN_MIN_LENGTH} characters).` };
  if (token.length > TOKEN_MAX_LENGTH) return { ok: false, reason: `This is too long for a MAST token (at most ${TOKEN_MAX_LENGTH} characters).` };
  return { ok: true, token };
}
