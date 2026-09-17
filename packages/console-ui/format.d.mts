/**
 * Short elapsed copy: `just now`, `42s`, `5m`, `3h`, `2d`, `4mo`, `1y`.
 * Accepts unix seconds, milliseconds (numbers below 1e12 are seconds),
 * numeric strings, ISO strings and `Date`s. Empty for unparsable input.
 */
export declare function formatRelative(input: number | string | Date, now?: number): string
/** `842ms`, `1.4s`, `2m 05s`, `1h 12m`. */
export declare function formatDuration(ms: number): string
/** Binary units with a space: `512 B`, `1.0 KiB`, `3.2 MiB`; `—` for null. */
export declare function formatBytes(n: number | null | undefined): string
/** `{ content: [...], details }` harness result envelope → `details`; a flat payload passes through. */
export declare function unwrapEnvelope(value: unknown): unknown
/** The innermost error code a rejected `iii.trigger` carries. */
export declare function errorCode(err: unknown): string | undefined
/** One readable string out of whatever a rejected `iii.trigger` throws — never `[object Object]`. */
export declare function errorMessage(err: unknown): string
/** Clipboard write with the insecure-origin textarea fallback; true on success, never throws. */
export declare function copyText(text: string): Promise<boolean>
