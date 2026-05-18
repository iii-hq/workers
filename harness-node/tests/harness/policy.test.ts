import { describe, expect, it } from 'vitest';
import { Permissions, decisionToValue } from '../../src/harness/policy.js';

const p = (yaml: string) => Permissions.parse(yaml);

describe('Permissions', () => {
  it('bare-string match allows with rule_id = function_id', () => {
    const perms = p(`
version: 1
rules:
  - shell::fs::ls
`);
    expect(perms.check('shell::fs::ls', {})).toEqual({
      kind: 'allow',
      rule_id: 'shell::fs::ls',
    });
  });

  it('explicit allow rule matches', () => {
    const perms = p(`
version: 1
rules:
  - rule_id: status/ok
    function: harness::status
    action: allow
`);
    expect(perms.check('harness::status', {})).toEqual({
      kind: 'allow',
      rule_id: 'status/ok',
    });
  });

  it('explicit deny rule matches', () => {
    const perms = p(`
version: 1
rules:
  - rule_id: rm/no
    function: shell::fs::rm
    action: deny
`);
    const d = perms.check('shell::fs::rm', {});
    expect(d.kind).toBe('deny');
    if (d.kind === 'deny') expect(d.rule_id).toBe('rm/no');
  });

  it('equals constraint respects action', () => {
    const perms = p(`
version: 1
rules:
  - rule_id: git/status
    function: shell::exec
    action: allow
    args:
      command:
        equals: "git status"
`);
    expect(perms.check('shell::exec', { command: 'git status' }).kind).toBe('allow');
    expect(perms.check('shell::exec', { command: 'git push' }).kind).toBe('needs_approval');
  });

  it('matches constraint respects action', () => {
    const perms = p(`
version: 1
rules:
  - rule_id: git/readonly
    function: shell::exec
    action: allow
    args:
      command:
        matches: "^git (status|log|diff)( |$)"
`);
    expect(perms.check('shell::exec', { command: 'git log -p' }).kind).toBe('allow');
    expect(perms.check('shell::exec', { command: 'git push' }).kind).toBe('needs_approval');
  });

  it('multiple args constraints are AND-combined', () => {
    const perms = p(`
version: 1
rules:
  - rule_id: state/agent-notes
    function: state::set
    action: allow
    args:
      scope:
        equals: agent
      key:
        matches: "^session/[a-z]+/notes$"
`);
    expect(perms.check('state::set', { scope: 'agent', key: 'session/abc/notes' }).kind).toBe(
      'allow',
    );
    expect(perms.check('state::set', { scope: 'global', key: 'session/abc/notes' }).kind).toBe(
      'needs_approval',
    );
    expect(perms.check('state::set', { scope: 'agent', key: 'global/secrets' }).kind).toBe(
      'needs_approval',
    );
  });

  it('first-match-wins: deny before broader allow', () => {
    const perms = p(`
version: 1
rules:
  - rule_id: git/no-force-push
    function: shell::exec
    action: deny
    args:
      command:
        matches: "^git push --force"
  - rule_id: git/all
    function: shell::exec
    action: allow
`);
    const d = perms.check('shell::exec', { command: 'git push --force' });
    expect(d.kind).toBe('deny');
    if (d.kind === 'deny') {
      expect(d.rule_id).toBe('git/no-force-push');
      expect(d.matched_constraint?.field).toBe('command');
    }
    expect(perms.check('shell::exec', { command: 'git status' }).kind).toBe('allow');
  });

  it('first-match-wins: allow before broader deny', () => {
    const perms = p(`
version: 1
rules:
  - rule_id: git/status-only
    function: shell::exec
    action: allow
    args:
      command:
        equals: "git status"
  - rule_id: git/forbid
    function: shell::exec
    action: deny
`);
    expect(perms.check('shell::exec', { command: 'git status' }).kind).toBe('allow');
    const d = perms.check('shell::exec', { command: 'git diff' });
    expect(d.kind).toBe('deny');
    if (d.kind === 'deny') expect(d.rule_id).toBe('git/forbid');
  });

  it('detailed rule missing rule_id is a load error', () => {
    expect(() =>
      p(`
version: 1
rules:
  - function: shell::exec
    action: deny
`),
    ).toThrow();
  });

  it('matches against non-string arg does not match', () => {
    const perms = p(`
version: 1
rules:
  - rule_id: t
    function: f
    action: allow
    args:
      field:
        matches: ".*"
`);
    expect(perms.check('f', { field: 123 }).kind).toBe('needs_approval');
  });

  it('invalid regex is a load error', () => {
    expect(() =>
      p(`
version: 1
rules:
  - rule_id: bad-regex
    function: f
    action: deny
    args:
      field:
        matches: "[unclosed"
`),
    ).toThrow();
  });

  it('empty permissions falls through for everything', () => {
    expect(Permissions.empty().check('anything', {}).kind).toBe('needs_approval');
  });
});

describe('decisionToValue', () => {
  it('emits allow shape', () => {
    expect(decisionToValue({ kind: 'allow', rule_id: 'r' })).toEqual({
      decision: 'allow',
      rule_id: 'r',
    });
  });
  it('emits deny shape with matched_constraint when present', () => {
    expect(
      decisionToValue({
        kind: 'deny',
        rule_id: 'r',
        matched_constraint: { field: 'command', operator: 'matches', value: '^x' },
      }),
    ).toEqual({
      decision: 'deny',
      rule_id: 'r',
      rule_action: 'deny',
      matched_constraint: { field: 'command', operator: 'matches', value: '^x' },
    });
  });
  it('emits needs_approval for fallthrough', () => {
    expect(decisionToValue({ kind: 'needs_approval' })).toEqual({
      decision: 'needs_approval',
    });
  });
});

describe('shipped iii-permissions.yaml default', () => {
  it('denies kernel surfaces', async () => {
    const { readFile } = await import('node:fs/promises');
    const text = await readFile('./iii-permissions.yaml', 'utf8');
    const perms = Permissions.parse(text);
    for (const fid of [
      'approval::resolve',
      'policy::check_permissions',
      'state::set',
      'state::update',
      'state::delete',
      'stream::set',
      'iii::durable::publish',
      'auth::set_token',
      'auth::delete_token',
      'run::start',
      'run::start_and_wait',
      'router::stream_assistant',
      'router::abort',
    ]) {
      const d = perms.check(fid, {});
      expect(d.kind).toBe('deny');
      if (d.kind === 'deny') expect(d.rule_id.startsWith('kernel/')).toBe(true);
    }
  });

  it('allows known read-only conveniences', async () => {
    const { readFile } = await import('node:fs/promises');
    const text = await readFile('./iii-permissions.yaml', 'utf8');
    const perms = Permissions.parse(text);
    for (const fid of [
      'harness::status',
      'state::get',
      'state::list',
      'models::list',
      'auth::status',
      'directory::engine::functions::list',
    ]) {
      expect(perms.check(fid, {}).kind).toBe('allow');
    }
  });
});
