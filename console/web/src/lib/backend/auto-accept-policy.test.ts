import { describe, expect, it } from 'vitest'
import {
  DEFAULT_POLICY,
  isAutoAcceptable,
  type AutoAcceptPolicy,
} from './auto-accept-policy'

describe('isAutoAcceptable (default policy)', () => {
  it('accepts plain read-style function ids', () => {
    expect(isAutoAcceptable('directory::skills::list')).toBe(true)
    expect(isAutoAcceptable('directory::skills::get')).toBe(true)
    expect(isAutoAcceptable('session-tree::messages')).toBe(true)
    expect(isAutoAcceptable('state::list')).toBe(true)
    expect(isAutoAcceptable('fs::ls')).toBe(true)
    expect(isAutoAcceptable('models::info')).toBe(true)
  })

  it('REGRESSION: does NOT deny safe reads under the shell:: worker namespace', () => {
    // Previous bug: a substring-match `shell` pattern denied every
    // function in the `shell::` worker, including pure reads like
    // `shell::fs::read` and `shell::fs::ls`. The fix makes the deny
    // boundary-anchored so the worker namespace alone doesn't trip
    // the policy — only the operation verb (`exec`, `run`, etc.)
    // does.
    expect(isAutoAcceptable('shell::fs::read')).toBe(true)
    expect(isAutoAcceptable('shell::fs::ls')).toBe(true)
    expect(isAutoAcceptable('shell::fs::stat')).toBe(true)
    expect(isAutoAcceptable('shell::env::get')).toBe(true)
  })

  it('still denies shell-namespaced operations that ARE state-mutating', () => {
    expect(isAutoAcceptable('shell::exec')).toBe(false)
    expect(isAutoAcceptable('shell::run')).toBe(false)
    expect(isAutoAcceptable('shell::fs::write')).toBe(false)
    expect(isAutoAcceptable('shell::fs::delete')).toBe(false)
  })

  it('rejects state-mutating function ids', () => {
    expect(isAutoAcceptable('fs::write')).toBe(false)
    expect(isAutoAcceptable('fs::write_file')).toBe(false)
    expect(isAutoAcceptable('shell::fs::write')).toBe(false)
    expect(isAutoAcceptable('db::update')).toBe(false)
    expect(isAutoAcceptable('storage::put')).toBe(false)
    expect(isAutoAcceptable('catalog::patch')).toBe(false)
    expect(isAutoAcceptable('state::set')).toBe(false)
    expect(isAutoAcceptable('config::create')).toBe(false)
  })

  it('rejects deletion / removal function ids', () => {
    expect(isAutoAcceptable('fs::delete')).toBe(false)
    expect(isAutoAcceptable('fs::rm')).toBe(false)
    expect(isAutoAcceptable('fs::remove')).toBe(false)
    expect(isAutoAcceptable('db::drop')).toBe(false)
    expect(isAutoAcceptable('cache::purge')).toBe(false)
    expect(isAutoAcceptable('runtime::destroy')).toBe(false)
  })

  it('rejects process-spawn / system-level function ids', () => {
    expect(isAutoAcceptable('sandbox::exec')).toBe(false)
    expect(isAutoAcceptable('proc::run')).toBe(false)
    expect(isAutoAcceptable('vm::eval')).toBe(false)
    expect(isAutoAcceptable('runtime::spawn')).toBe(false)
    expect(isAutoAcceptable('proc::kill')).toBe(false)
    expect(isAutoAcceptable('container::run_command')).toBe(false)
  })

  it('rejects external egress function ids', () => {
    expect(isAutoAcceptable('mail::send')).toBe(false)
    expect(isAutoAcceptable('http::post')).toBe(false)
    expect(isAutoAcceptable('iii::durable::publish')).toBe(false)
    expect(isAutoAcceptable('alerts::notify')).toBe(false)
    expect(isAutoAcceptable('integrations::webhook')).toBe(false)
  })

  it('rejects deploy / install / push function ids', () => {
    expect(isAutoAcceptable('pkg::install')).toBe(false)
    expect(isAutoAcceptable('pkg::uninstall')).toBe(false)
    expect(isAutoAcceptable('release::deploy')).toBe(false)
    expect(isAutoAcceptable('git::push')).toBe(false)
    expect(isAutoAcceptable('github::pull_request')).toBe(false)
  })

  it('rejects recursive-dispatch function ids', () => {
    expect(isAutoAcceptable('agent::trigger')).toBe(false)
    expect(isAutoAcceptable('orchestrator::dispatch')).toBe(false)
  })

  it('rejects credential / secret surfaces', () => {
    expect(isAutoAcceptable('vault::secret')).toBe(false)
    expect(isAutoAcceptable('auth::get_token')).toBe(false)
    expect(isAutoAcceptable('creds::credential::lookup')).toBe(false)
  })

  it('fails closed on empty / undefined ids', () => {
    expect(isAutoAcceptable(undefined)).toBe(false)
    expect(isAutoAcceptable('')).toBe(false)
  })

  it('does NOT match keyword substrings inside larger words', () => {
    // metadata::is_writeable contains "write" inside a larger word.
    // The boundary-anchored pattern should let this through.
    expect(isAutoAcceptable('metadata::is_writeable')).toBe(true)
    expect(isAutoAcceptable('catalog::set_index')).toBe(false) // set IS a boundary match
    expect(isAutoAcceptable('catalog::settings_index')).toBe(true) // settings ≠ set
    expect(isAutoAcceptable('runtime::overrun')).toBe(true) // "run" inside "overrun" not boundary-matched (no run keyword anyway)
  })
})

describe('isAutoAcceptable (custom policy)', () => {
  it('honors a tighter custom policy', () => {
    const strict: AutoAcceptPolicy = {
      denyPatterns: [/.*/], // deny everything
      denyExact: new Set(),
    }
    expect(isAutoAcceptable('fs::ls', strict)).toBe(false)
  })

  it('honors a wider custom policy that strips most defaults', () => {
    const permissive: AutoAcceptPolicy = {
      denyPatterns: [],
      denyExact: new Set(['agent::trigger']),
    }
    expect(isAutoAcceptable('fs::write', permissive)).toBe(true)
    expect(isAutoAcceptable('agent::trigger', permissive)).toBe(false)
  })
})

describe('DEFAULT_POLICY shape', () => {
  it('exposes a non-empty deny surface', () => {
    expect(DEFAULT_POLICY.denyPatterns.length).toBeGreaterThan(0)
    expect(DEFAULT_POLICY.denyExact.size).toBeGreaterThan(0)
  })
})
