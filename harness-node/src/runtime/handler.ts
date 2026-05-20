/**
 * Helpers for parsing function payloads.
 *
 * The iii bus delivers payloads as plain JSON. Many engine entry-points
 * historically wrap user input as `{ body: { ... } }`, so the helper
 * `unwrapBody` peels the body envelope when present and falls back to
 * the raw payload otherwise — mirroring the Rust pattern at e.g.
 * `harness/src/lib.rs:142` (`input.get("body").cloned().unwrap_or(input)`).
 */

export function unwrapBody(input: unknown): Record<string, unknown> {
  if (typeof input !== 'object' || input === null) return {};
  const obj = input as Record<string, unknown>;
  if (
    Object.prototype.hasOwnProperty.call(obj, 'body') &&
    obj.body !== null &&
    typeof obj.body === 'object'
  ) {
    return obj.body as Record<string, unknown>;
  }
  return obj;
}

/** Helper that throws an Error with a stable message on missing required fields. */
export function requireString(obj: Record<string, unknown>, key: string): string {
  const v = obj[key];
  if (typeof v !== 'string' || v.length === 0) {
    throw new Error(`missing required string field: ${key}`);
  }
  return v;
}

export function optionalString(obj: Record<string, unknown>, key: string): string | undefined {
  const v = obj[key];
  return typeof v === 'string' ? v : undefined;
}
