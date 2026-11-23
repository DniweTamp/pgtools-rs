/** UTF-8 aware string helpers for fixed-width binary name fields. */

export function utf8ByteLength(value: string): number {
  return new TextEncoder().encode(value).length;
}

/**
 * Truncate `value` so its UTF-8 encoding fits in `maxBytes`, never splitting a code point.
 * The struct formats store names in fixed-size, NUL-terminated fields, and the Rust writers
 * copy bytes verbatim — so an over-long name is silently cut (possibly mid-character) unless
 * the editor enforces the limit.
 */
export function clampUtf8Bytes(value: string, maxBytes: number): string {
  if (utf8ByteLength(value) <= maxBytes) return value;
  let out = "";
  let used = 0;
  for (const ch of value) {
    const bytes = utf8ByteLength(ch);
    if (used + bytes > maxBytes) break;
    out += ch;
    used += bytes;
  }
  return out;
}
