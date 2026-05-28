import { describe, expect, it } from 'vitest'
import type { FunctionEntry } from '@/lib/functions'
import { filterAllowlistCandidates } from './allowlist-filter'

function entry(id: string): FunctionEntry {
  return { id, description: '' }
}

describe('filterAllowlistCandidates', () => {
  it('keeps tool-like functions (Tier 0)', () => {
    const input = [
      entry('shell::fs::ls'),
      entry('shell::fs::write'),
      entry('shell::exec'),
      entry('sandbox::create'),
      entry('fs::read'),
      entry('mytool::do_thing'),
    ]
    const out = filterAllowlistCandidates(input).map((e) => e.id)
    expect(out).toEqual(input.map((e) => e.id))
  })

  it('hides Tier 1 orchestration plumbing', () => {
    const input = [
      entry('state::set'),
      entry('state::get'),
      entry('policy::check_permissions'),
      entry('turn::function_execute'),
      entry('session::ensure'),
      entry('session-tree::compactions'),
      entry('session-inbox::push'),
      entry('approval::resolve'),
      entry('approval::set_mode'),
      entry('hook-fanout::publish_collect'),
      entry('iii::durable::publish'),
      entry('ui::subscribe'),
      entry('ui::session::event'),
    ]
    expect(filterAllowlistCandidates(input)).toEqual([])
  })

  it('hides Tier 2 runtime services + provider workers', () => {
    const input = [
      entry('models-catalog::list'),
      entry('llm-budget::record'),
      entry('context-compaction::compact_session'),
      entry('engine::echo'),
      entry('functions::list'),
      entry('directory::index'),
      entry('trigger::run'),
      entry('auth::status'),
      entry('provider-anthropic::stream'),
      entry('provider-openai::config'),
    ]
    expect(filterAllowlistCandidates(input)).toEqual([])
  })

  it('hides agent lifecycle events but keeps agent::trigger visible', () => {
    const input = [
      entry('agent::trigger'),
      entry('agent::before_function_call'),
      entry('agent::after_function_call'),
      entry('agent::turn_end'),
      entry('agent::run_start'),
    ]
    const out = filterAllowlistCandidates(input).map((e) => e.id)
    expect(out).toEqual(['agent::trigger'])
  })

  it('does not partial-match a prefix (e.g. authorize-* is NOT auth::)', () => {
    // sanity check the boundary: prefix match is on `auth::`, not `auth`,
    // so an unrelated namespace starting with the same word survives.
    const input = [entry('authorize::do_thing')]
    expect(filterAllowlistCandidates(input)).toHaveLength(1)
  })
})
