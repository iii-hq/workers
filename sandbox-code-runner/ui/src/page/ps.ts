/**
 * `ps` output parsing for the console tab's process table. The preset
 * images ship busybox (`PID USER TIME COMMAND`), but a custom image may
 * carry procps (`PID TTY TIME CMD`, or the `ps -ef` UID-first shape) — so
 * the header locates the PID and command columns instead of the parser
 * assuming positions. A header it cannot place returns null and the
 * caller falls back to the raw text.
 */

export interface PsProcess {
  pid: number
  cmd: string
}

export function parsePsOutput(text: string): PsProcess[] | null {
  const lines = text.split('\n').filter((line) => line.trim().length > 0)
  if (lines.length === 0) return null
  const header = lines[0]
    .trim()
    .split(/\s+/)
    .map((token) => token.toUpperCase())
  const pidIdx = header.indexOf('PID')
  const cmdIdx = header.findIndex((token) => token === 'CMD' || token === 'COMMAND')
  // The command must be the LAST column — every ps shape puts it there,
  // which is what lets a row parser join "everything from that column on".
  if (pidIdx < 0 || cmdIdx !== header.length - 1) return null
  const out: PsProcess[] = []
  for (const line of lines.slice(1)) {
    const tokens = line.trim().split(/\s+/)
    if (tokens.length <= cmdIdx) continue
    const pid = Number(tokens[pidIdx])
    if (!Number.isInteger(pid) || pid <= 0) continue
    out.push({ pid, cmd: tokens.slice(cmdIdx).join(' ') })
  }
  return out
}
