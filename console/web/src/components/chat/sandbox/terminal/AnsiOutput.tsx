import { cn } from '@/lib/utils'

interface AnsiOutputProps {
  stdout?: string
  stderr?: string
  className?: string
}

/**
 * Stream-coloured stdout/stderr pane. Sandbox exec is buffered (see
 * `adapters.rs::run` — no incremental writes are surfaced), so we
 * just stack stdout above stderr with distinct tones rather than
 * try to interleave by timestamp (which the wire shape doesn't
 * carry). ANSI escape parsing is explicitly out of scope per the
 * plan; we strip the escape introducer to avoid leaking the raw
 * `\x1b[` bytes into the rendered text.
 */
export function AnsiOutput({ stdout, stderr, className }: AnsiOutputProps) {
  const hasStdout = !!stdout && stdout.length > 0
  const hasStderr = !!stderr && stderr.length > 0

  if (!hasStdout && !hasStderr) {
    return (
      <div
        className={cn(
          'bg-bg px-3 py-3 font-mono text-[12.5px] leading-[1.55] text-ink-ghost',
          className,
        )}
      >
        · no output
      </div>
    )
  }

  return (
    <div
      className={cn(
        'bg-bg px-3 py-2 font-mono text-[12.5px] leading-[1.55]',
        className,
      )}
    >
      {hasStdout ? (
        <pre className="text-ink whitespace-pre-wrap break-words m-0">
          <code>{stripAnsi(stdout ?? '')}</code>
        </pre>
      ) : null}
      {hasStdout && hasStderr ? <div className="h-1" /> : null}
      {hasStderr ? (
        <pre className="text-warn whitespace-pre-wrap break-words m-0">
          <code>{stripAnsi(stderr ?? '')}</code>
        </pre>
      ) : null}
    </div>
  )
}

/* Single-pass strip of CSI/OSC escape sequences. Keeps the visible
   text intact (newlines, tabs) so monospace alignment in tools like
   `ls -l` / `cargo test` survives. The two control-char ranges are
   intentional: 0x1B is the ANSI ESC introducer (CSI/OSC) and 0x07 is
   the BEL terminator for OSC sequences. */
// biome-ignore lint/suspicious/noControlCharactersInRegex: stripping ANSI escape sequences.
const CSI_REGEX = /\x1b\[[0-9;?]*[ -/]*[@-~]/g
// biome-ignore lint/suspicious/noControlCharactersInRegex: stripping ANSI OSC (ESC ] … BEL) sequences.
const OSC_REGEX = /\x1b\][^\x07]*\x07/g

function stripAnsi(s: string): string {
  return s.replace(CSI_REGEX, '').replace(OSC_REGEX, '')
}
