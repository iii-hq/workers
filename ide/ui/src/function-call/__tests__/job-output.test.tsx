import type { Host } from '@iii-dev/console-ui'
import type { ReactNode } from 'react'
import type { JobRecord } from '../parsers'
import { renderToStaticMarkup } from 'react-dom/server'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@iii-dev/console-ui', () => {
  const tag = ({ children }: { children?: ReactNode }) => <span>{children}</span>
  return {
    Badge: tag,
    Button: ({ children }: { children?: ReactNode }) => <button type="button">{children}</button>,
    Chip: tag,
    EmptyState: ({ title }: { title: string }) => <span>{title}</span>,
    MetaRow: tag,
    TerminalCommandLine: tag,
    TerminalStream: ({ text }: { text: string }) => <pre>{text}</pre>,
    uiClasses: { pulse: 'pulse' },
  }
})

import { JobOutputBody, liveTail, pollJob, ShellExecBgView } from '../ExecBgView'

const job = (status: string, stdout: string) => ({
  id: 'job-1',
  argv: ['cargo', 'test'],
  started_at_ms: 1,
  finished_at_ms: status === 'running' ? null : 2,
  status,
  exit_code: status === 'running' ? null : 0,
  stdout,
  stderr: '',
  stdout_truncated: false,
  stderr_truncated: false,
})

function hostAnswering(...answers: unknown[]) {
  const call = vi.fn(async () => {
    const answer = answers.shift()
    if (answer instanceof Error) throw answer
    return answer
  })
  return { host: { iii: { call } } as unknown as Host, call }
}

describe('pollJob', () => {
  beforeEach(() => {
    vi.useFakeTimers()
  })
  afterEach(() => {
    vi.useRealTimers()
  })

  it('reads a running job until it ends', async () => {
    const { host, call } = hostAnswering(
      { job: job('running', 'first\n') },
      { job: job('finished', 'first\nsecond\n') },
    )
    const seen: string[] = []
    pollJob(host, 'job-1', (record) => seen.push(record.stdout), vi.fn())
    await vi.advanceTimersByTimeAsync(0)
    expect(call).toHaveBeenCalledWith('shell::status', { job_id: 'job-1' })
    expect(seen).toEqual(['first\n'])
    await vi.advanceTimersByTimeAsync(2_000)
    expect(seen).toEqual(['first\n', 'first\nsecond\n'])
    await vi.advanceTimersByTimeAsync(20_000)
    expect(call).toHaveBeenCalledTimes(2)
  })

  it('keeps reading a killed job for as long as a drain can delay its output', async () => {
    const reads = Array.from({ length: 6 }, () => ({ job: job('killed', 'tail\n') }))
    const { host, call } = hostAnswering(...reads, { job: job('killed', 'all of it\n') })
    const seen: string[] = []
    pollJob(host, 'job-1', (record) => seen.push(record.stdout), vi.fn())
    await vi.advanceTimersByTimeAsync(12_000)
    expect(seen.at(-1)).toBe('all of it\n')
    await vi.advanceTimersByTimeAsync(20_000)
    expect(call).toHaveBeenCalledTimes(7)
  })

  it('does not read while the tab is hidden', async () => {
    vi.stubGlobal('document', { hidden: true })
    const { host, call } = hostAnswering({ job: job('finished', '') }, { job: job('finished', '') })
    const stop = pollJob(host, 'job-1', vi.fn(), vi.fn())
    await vi.advanceTimersByTimeAsync(10_000)
    expect(call).not.toHaveBeenCalled()
    vi.stubGlobal('document', { hidden: false })
    await vi.advanceTimersByTimeAsync(2_000)
    expect(call).toHaveBeenCalledTimes(1)
    stop()
    vi.unstubAllGlobals()
  })

  it('reports a job that aged out and stops', async () => {
    const { host, call } = hostAnswering(new Error('no such job: job-1'))
    const onError = vi.fn()
    pollJob(host, 'job-1', vi.fn(), onError)
    await vi.advanceTimersByTimeAsync(10_000)
    expect(onError).toHaveBeenCalledWith(expect.stringContaining('no such job'))
    expect(call).toHaveBeenCalledTimes(1)
  })

  it('reads nothing more once stopped', async () => {
    const { host, call } = hostAnswering({ job: job('running', '') })
    const stop = pollJob(host, 'job-1', vi.fn(), vi.fn())
    await vi.advanceTimersByTimeAsync(0)
    stop()
    await vi.advanceTimersByTimeAsync(10_000)
    expect(call).toHaveBeenCalledTimes(1)
  })
})

describe('liveTail', () => {
  it('keeps the newest lines, which the stream pane would otherwise cut', () => {
    const text = `${Array.from({ length: 40 }, (_, i) => `line-${i + 1}`).join('\n')}\n`
    const tail = liveTail(text)
    expect(tail.split('\n')).toHaveLength(6)
    expect(tail.startsWith('line-35')).toBe(true)
    expect(tail.endsWith('line-40')).toBe(true)
  })

  it('keeps the end of one very long line', () => {
    const tail = liveTail(`${'x'.repeat(5_000)}end`)
    expect(tail).toHaveLength(2_000)
    expect(tail.endsWith('end')).toBe(true)
  })
})

describe('JobOutputBody', () => {
  const body = (status: string, stdout: string, stale = false) =>
    renderToStaticMarkup(
      <JobOutputBody job={job(status, stdout) as JobRecord} stale={stale} />,
    )
  const lines = Array.from({ length: 40 }, (_, i) => `line-${i + 1}`).join('\n')

  it('shows a running job its newest lines and says so', () => {
    const html = body('running', lines)
    expect(html).toContain('line-40')
    expect(html).not.toContain('line-34')
    expect(html).toContain('Latest lines')
  })

  it('shows a finished job all of its output', () => {
    const html = body('finished', lines)
    expect(html).toContain('line-1\n')
    expect(html).toContain('line-40')
    expect(html).not.toContain('Latest lines')
  })

  it('has one message for a running job with nothing to show', () => {
    const html = body('running', '\n')
    expect(html).toContain('No output yet.')
    expect(html).not.toContain('No output<')
    expect(html).not.toContain('Latest lines')
  })

  it('promises nothing and shows no status after a failed read', () => {
    const html = body('running', lines, true)
    expect(html).not.toContain('Latest lines')
    expect(html).not.toContain('running</span>')
    expect(html).toContain('line-40')
  })
})

describe('ShellExecBgView', () => {
  it('offers the output of a started job', () => {
    const { host } = hostAnswering()
    const html = renderToStaticMarkup(
      <ShellExecBgView
        host={host}
        input={{ command: 'cargo test' }}
        output={{ job_id: 'job-1', argv: ['cargo', 'test'] }}
      />,
    )
    expect(html).toContain('Show output')
  })

  it('has nothing to show before the job has started', () => {
    const { host } = hostAnswering()
    const html = renderToStaticMarkup(
      <ShellExecBgView host={host} input={{ command: 'cargo test' }} output={null} running />,
    )
    expect(html).not.toContain('Show output')
  })
})
