/**
 * A function call as an `iii call` command line. Pure, with no React or host,
 * so cli-command.test.mjs can load it under `node --test`.
 *
 * `iii call` reads each `key=value` value as JSON first and keeps it as a
 * plain string only when it does not parse: `id=42` sends the integer 42,
 * `id='"42"'` sends the string "42" (checked against the 0.26 engine). The
 * command it is pasted into is a POSIX shell, so every token that holds
 * anything a shell would act on is single-quoted.
 */

/** One `key=value` argument of an `iii call`, with what kind of value it is. */
export interface CliArg {
  /** Empty when the key itself needed quoting: `value` then holds the whole token. */
  key: string
  /** The value as typed on the command line, quotes included. */
  value: string
  kind: 'string' | 'number' | 'keyword' | 'json'
}

/** Characters no POSIX shell expands, splits on or redirects with. */
const SHELL_SAFE = /^[A-Za-z0-9_@%+,.:/=-]+$/
/** A key `iii call` reads as one name, before the first `=`. */
const KEY_SAFE = /^[A-Za-z0-9_.-]+$/

/** A shell word for `text`: bare when every character is safe, else single-quoted. */
export function shellWord(text: string): string {
  if (SHELL_SAFE.test(text)) return text
  // A single quote cannot appear inside single quotes: close the segment,
  // emit an escaped quote, reopen.
  return `'${text.replaceAll("'", `'\\''`)}'`
}

/** True when `iii call` would read this string as something other than itself. */
function readsAsJson(text: string): boolean {
  try {
    JSON.parse(text)
    return true
  } catch {
    return false
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/**
 * The body as `iii call` arguments. A string stays as written unless the
 * CLI would parse it into another type (`"42"`, `"true"`, `"null"`,
 * `"[1]"`), in which case it goes as JSON so it arrives as the same string.
 * Everything else goes as JSON. Empty values keep their quotes (`key=''`).
 */
export function cliArgs(payload: unknown): CliArg[] {
  if (!isRecord(payload)) return []
  return Object.entries(payload).map(([key, raw]) => {
    const kind: CliArg['kind'] =
      typeof raw === 'string'
        ? 'string'
        : typeof raw === 'number'
          ? 'number'
          : raw === null || typeof raw === 'boolean'
            ? 'keyword'
            : 'json'
    const literal =
      typeof raw === 'string' ? (readsAsJson(raw) ? JSON.stringify(raw) : raw) : (JSON.stringify(raw) ?? '')
    if (!KEY_SAFE.test(key)) return { key: '', value: shellWord(`${key}=${literal}`), kind }
    return { key, value: shellWord(literal), kind }
  })
}

/** One argument as it reads on the command line. */
export function cliToken(arg: CliArg): string {
  return arg.key ? `${arg.key}=${arg.value}` : arg.value
}

/** The same call as one CLI line, ready to paste. */
export function asCliCommand(functionId: string, payload: unknown): string {
  const args = cliArgs(payload).map(cliToken)
  return ['iii', 'call', shellWord(functionId), ...args].join(' ')
}
