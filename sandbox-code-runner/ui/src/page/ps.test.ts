/**
 * The ps parser against the header shapes the console tab meets in the
 * wild: busybox (preset images), procps `ps` and `ps -ef` (custom
 * images), and malformed output — which must return null so the caller
 * falls back to the raw text.
 */

import { describe, expect, it } from 'vitest'
import { parsePsOutput } from './ps'

const BUSYBOX = `PID   USER     TIME  COMMAND
    1 root      0:00 sleep infinity
   12 root      0:00 sh -c ps
   13 root      0:00 ps
`

const PROCPS = `  PID TTY          TIME CMD
    1 ?        00:00:00 sleep
   40 pts/0    00:00:00 ps
`

const PS_EF = `UID        PID  PPID  C STIME TTY          TIME CMD
root         1     0  0 10:00 ?        00:00:00 sleep infinity
root        55     1  0 10:02 ?        00:00:00 node server.js
`

describe('parsePsOutput', () => {
  it('reads the busybox shape, command joined whole', () => {
    expect(parsePsOutput(BUSYBOX)).toEqual([
      { pid: 1, cmd: 'sleep infinity' },
      { pid: 12, cmd: 'sh -c ps' },
      { pid: 13, cmd: 'ps' },
    ])
  })

  it('reads the procps shape', () => {
    expect(parsePsOutput(PROCPS)).toEqual([
      { pid: 1, cmd: 'sleep' },
      { pid: 40, cmd: 'ps' },
    ])
  })

  it('reads the ps -ef shape — PID is not the first column', () => {
    expect(parsePsOutput(PS_EF)).toEqual([
      { pid: 1, cmd: 'sleep infinity' },
      { pid: 55, cmd: 'node server.js' },
    ])
  })

  it('skips rows whose PID slot is not a positive integer', () => {
    const out = parsePsOutput(`PID USER TIME COMMAND
  junk row without numbers here
  9 root 0:00 top
`)
    expect(out).toEqual([{ pid: 9, cmd: 'top' }])
  })

  it('returns null when the header cannot be placed (raw fallback)', () => {
    expect(parsePsOutput('')).toBeNull()
    expect(parsePsOutput('ps: not found\n')).toBeNull()
    expect(parsePsOutput('total 12\n-rw-r--r-- 1 root root 0 x\n')).toBeNull()
    // A command column that is not last would mis-join rows — refuse it.
    expect(parsePsOutput('PID COMMAND USER\n1 sleep root\n')).toBeNull()
  })
})
