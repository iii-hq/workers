/**
 * Read/write deployment permission defaults from the canonical
 * `approval-gate` configuration entry (single source — no localStorage).
 */

import type { PermissionMode } from '@/lib/backend/approval-settings'
import type { HarnessFunctionPolicy } from '@/lib/backend/harness-send'
import {
  getConfiguration,
  type JsonValue,
  setConfiguration,
} from '@/pages/Configuration/tabs/WorkersTab/api'

export const APPROVAL_GATE_CONFIG_ID = 'approval-gate'

export interface ApprovalGateConfigView {
  default_mode: PermissionMode
  rules: JsonValue[]
}

interface StructuredRule {
  function?: string
  action?: string
  modes?: string[]
}

function isPermissionMode(v: unknown): v is PermissionMode {
  return v === 'manual' || v === 'auto' || v === 'full'
}

function asRulesArray(value: JsonValue | undefined): JsonValue[] {
  return Array.isArray(value) ? value : []
}

/** Extract auto-mode trust globs from mode-scoped allow rules. */
export function autoAllowSeedFromRules(rules: JsonValue[]): string[] {
  const out: string[] = []
  for (const entry of rules) {
    if (typeof entry === 'string') continue
    if (!entry || typeof entry !== 'object' || Array.isArray(entry)) continue
    const rule = entry as StructuredRule
    if (rule.action !== 'allow') continue
    if (!rule.modes?.includes('auto')) continue
    if (typeof rule.function === 'string' && rule.function.length > 0) {
      out.push(rule.function)
    }
  }
  return out.sort()
}

/** Derive the harness structural floor from deployment rules. */
export function deriveFunctionPolicy(
  rules: JsonValue[],
): HarnessFunctionPolicy {
  const deny = new Set<string>(['approval::*', 'configuration::*'])
  for (const entry of rules) {
    if (typeof entry === 'string') {
      if (entry.startsWith('!')) deny.add(entry.slice(1))
      continue
    }
    if (!entry || typeof entry !== 'object' || Array.isArray(entry)) continue
    const rule = entry as StructuredRule
    if (rule.action === 'deny' && typeof rule.function === 'string') {
      deny.add(rule.function)
    }
  }
  return {
    allow: ['*'],
    deny: Array.from(deny).sort(),
    expose: 'agent_trigger',
  }
}

export async function loadApprovalGateConfig(): Promise<ApprovalGateConfigView> {
  const raw = await getConfiguration(APPROVAL_GATE_CONFIG_ID)
  const obj =
    raw && typeof raw === 'object' && !Array.isArray(raw)
      ? (raw as Record<string, JsonValue>)
      : {}
  const default_mode = isPermissionMode(obj.default_mode)
    ? obj.default_mode
    : 'manual'
  return {
    default_mode,
    rules: asRulesArray(obj.rules),
  }
}

function withoutAutoSeedRules(rules: JsonValue[]): JsonValue[] {
  return rules.filter((entry) => {
    if (typeof entry === 'string') return true
    if (!entry || typeof entry !== 'object' || Array.isArray(entry)) return true
    const rule = entry as StructuredRule
    if (rule.action !== 'allow') return true
    return !(rule.modes?.length === 1 && rule.modes[0] === 'auto')
  })
}

/** Persist default mode + auto allowlist into the deployment rules list. */
export async function saveApprovalGateDefaults(
  defaultMode: PermissionMode,
  allowlist: string[],
): Promise<ApprovalGateConfigView> {
  const current = await loadApprovalGateConfig()
  const baseRules = withoutAutoSeedRules(current.rules)
  const seedRules: JsonValue[] = allowlist.map((function_id) => ({
    function: function_id,
    action: 'allow',
    modes: ['auto'],
  }))
  const nextRules = [...baseRules, ...seedRules]
  const payload = {
    default_mode: defaultMode,
    rules: nextRules,
  }
  await setConfiguration({ id: APPROVAL_GATE_CONFIG_ID, value: payload })
  return {
    default_mode: defaultMode,
    rules: nextRules,
  }
}

export async function loadApprovalGateDefaults(): Promise<{
  defaultMode: PermissionMode
  allowlist: string[]
  functionPolicy: HarnessFunctionPolicy
}> {
  const cfg = await loadApprovalGateConfig()
  return {
    defaultMode: cfg.default_mode,
    allowlist: autoAllowSeedFromRules(cfg.rules),
    functionPolicy: deriveFunctionPolicy(cfg.rules),
  }
}
