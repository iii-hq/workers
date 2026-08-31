import { describe, expect, it } from 'vitest'
import {
  formatArgv,
  formatEpochMs,
  formatShellCommand,
  jobDurationMs,
  jobStatusPill,
} from '../format'

describe('formatShellCommand', () => {
  it('renders the shell line verbatim when args are absent', () => {
    expect(formatShellCommand({ command: 'echo "hi there" | wc -l' })).toBe(
      'echo "hi there" | wc -l',
    )
  })

  it('treats null args the same as absent', () => {
    expect(formatShellCommand({ command: 'ls -la /tmp', args: null })).toBe(
      'ls -la /tmp',
    )
  })

  it('quote-joins the argv when args are present', () => {
    expect(
      formatShellCommand({ command: 'echo', args: ['hi there', '$HOME'] }),
    ).toBe("echo 'hi there' '$HOME'")
  })

  it('keeps the []-vs-absent wire distinction visible', () => {
    // Absent ⇒ shell line, displayed verbatim.
    expect(formatShellCommand({ command: 'my program' })).toBe('my program')
    // Present (even empty) ⇒ verbatim argv: the program slot gets quoted.
    expect(formatShellCommand({ command: 'my program', args: [] })).toBe(
      "'my program'",
    )
  })
})

describe('formatArgv', () => {
  it('quote-joins a server-resolved argv into a paste-able line', () => {
    expect(formatArgv(['node', '--eval', 'console.log(1)'])).toBe(
      "node --eval 'console.log(1)'",
    )
  })

  it('renders an empty argv as an empty string', () => {
    expect(formatArgv([])).toBe('')
  })
})

describe('jobStatusPill', () => {
  it('maps every JobStatus to its tone', () => {
    expect(jobStatusPill('running')).toEqual({
      label: 'running',
      tone: 'accent',
    })
    expect(jobStatusPill('finished')).toEqual({
      label: 'finished',
      tone: 'default',
    })
    expect(jobStatusPill('killed')).toEqual({ label: 'killed', tone: 'warn' })
    expect(jobStatusPill('failed')).toEqual({ label: 'failed', tone: 'alert' })
  })
})

describe('jobDurationMs', () => {
  it('returns the wall-clock delta for finished jobs', () => {
    expect(jobDurationMs({ started_at_ms: 1_000, finished_at_ms: 4_500 })).toBe(
      3_500,
    )
  })

  it('returns null while the job is still running', () => {
    expect(
      jobDurationMs({ started_at_ms: 1_000, finished_at_ms: null }),
    ).toBeNull()
  })

  it('clamps negative clock skew to 0', () => {
    expect(jobDurationMs({ started_at_ms: 2_000, finished_at_ms: 1_000 })).toBe(
      0,
    )
  })
})

describe('formatEpochMs', () => {
  it('treats epoch 0 as unknown', () => {
    expect(formatEpochMs(0)).toBe('—')
  })

  it('floors millis to seconds before delegating to formatMtime', () => {
    expect(formatEpochMs(Date.now())).toBe('just now')
    expect(formatEpochMs(Date.now() - 120_000)).toBe('2m ago')
  })
})
