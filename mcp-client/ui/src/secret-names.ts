const NAME = /^(authorization|cookie|proxy-authorization)$/i
// No `u` flag: case folding stays ASCII-only, like Rust's `eq_ignore_ascii_case`.
const WORD = /^(key|apikey|token|secret|password|passwd|pat|auth|credential|credentials)$/i

/** Whether a header or env name probably carries a credential: the twin of
 * `looks_secret_name` in `mcp-client/src/refs.rs`, tested on the same vector. */
export function looksSecretName(name: string): boolean {
  return NAME.test(name) || name.split(/[-_]/).some((segment) => WORD.test(segment))
}
