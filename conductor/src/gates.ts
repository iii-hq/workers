import type { ISdk } from 'iii-sdk'
import type { GateSpec } from './types.js'

export interface GateOutcome {
  ok: boolean
  reason?: string
}

export async function runGate(sdk: ISdk, gate: GateSpec, cwd: string): Promise<GateOutcome> {
  try {
    const result = await sdk.trigger<{ cwd: string }, GateOutcome | { ok: boolean; reason?: string }>({
      function_id: gate.function_id,
      payload: { cwd },
      timeoutMs: 600_000,
    })
    if (typeof result === 'object' && result !== null && 'ok' in result) {
      return { ok: Boolean(result.ok), reason: result.reason }
    }
    return { ok: false, reason: 'gate returned non-object' }
  } catch (err) {
    return { ok: false, reason: String(err) }
  }
}

export async function runAllGates(sdk: ISdk, gates: GateSpec[], cwd: string): Promise<Record<string, GateOutcome>> {
  const out: Record<string, GateOutcome> = {}
  for (const gate of gates) {
    out[gate.function_id] = await runGate(sdk, gate, cwd)
  }
  return out
}

export function allPassed(results: Record<string, GateOutcome>): boolean {
  return Object.values(results).every((r) => r.ok)
}
