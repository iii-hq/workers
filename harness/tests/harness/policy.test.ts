/**
 * Adversarial tests for the permission engine — the harness's security
 * floor. The governing question for every test is: can crafted input get an
 * `allow` it should not, evade a `deny`, or crash / fail OPEN the evaluator?
 *
 * The engine's contract is fail-CLOSED: anything it cannot positively match
 * to an allow/deny rule resolves to `needs_approval` (a human gate). These
 * tests lock that in, and document two rule-AUTHORING footguns the engine
 * faithfully (and dangerously) honours.
 */
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterAll, describe, expect, it } from 'vitest';
import { decisionToValue, registerPolicy } from '../../src/harness/policy/check-permissions.js';
import type { PermissionsHandle } from '../../src/harness/policy/handle.js';
import { PermissionsHandle as RealPermissionsHandle } from '../../src/harness/policy/handle.js';
import { Permissions } from '../../src/harness/policy/permissions.js';
import type { Decision } from '../../src/harness/policy/types.js';

const p = (yaml: string) => Permissions.parse(yaml);
const kind = (yaml: string, fid: string, args: unknown) => p(yaml).check(fid, args).kind;

describe('malformed rules are rejected wholesale (no half-loaded policy)', () => {
  it.each([
    ['empty deny shorthand "!"', 'rules:\n  - "!"'],
    ['object missing function', 'rules:\n  - action: allow'],
    ['object with bad action', 'rules:\n  - function: f\n    action: maybe'],
    [
      'invalid regex constraint',
      'rules:\n  - function: f\n    action: deny\n    args:\n      x:\n        matches: "[unclosed"',
    ],
    [
      'unknown constraint shape',
      'rules:\n  - function: f\n    action: allow\n    args:\n      x:\n        gt: 1',
    ],
    ['null list entry', 'rules:\n  - ~'],
  ])('throws on %s', (_label, yaml) => {
    expect(() => p(`version: 1\n${yaml}\n`)).toThrow();
  });
});

describe('malformed policy *files* fail closed, never open', () => {
  it.each([
    ['rules is not a list', 'version: 1\nrules: nope'],
    ['rules omitted entirely', 'version: 1'],
    ['top-level is a list', '- a\n- b'],
    ['empty document', ''],
  ])('%s → zero rules → needs_approval for everything', (_label, yaml) => {
    const perms = p(yaml);
    expect(perms.ruleCount()).toBe(0);
    expect(perms.check('anything', {})).toEqual({ kind: 'needs_approval' });
  });

  it('empty permissions gates every call', () => {
    expect(Permissions.empty().check('shell::exec', { command: 'rm -rf /' }).kind).toBe(
      'needs_approval',
    );
  });
});

describe('fail-closed: nothing slips into allow by type confusion or odd args', () => {
  const allowExactNumber = `
version: 1
rules:
  - function: f
    action: allow
    args:
      n:
        equals: 0
`;
  const allowMatch = `
version: 1
rules:
  - function: f
    action: allow
    args:
      cmd:
        matches: ".+"
`;

  it('equals is strict — string "0" does not satisfy numeric 0 and vice versa', () => {
    const allowStr = `
version: 1
rules:
  - function: f
    action: allow
    args:
      n:
        equals: "0"
`;
    expect(kind(allowExactNumber, 'f', { n: 0 })).toBe('allow');
    expect(kind(allowExactNumber, 'f', { n: '0' })).toBe('needs_approval');
    expect(kind(allowStr, 'f', { n: 0 })).toBe('needs_approval');
  });

  it('matches only fires for string actuals — numbers/booleans/null do not', () => {
    for (const v of [123, true, null, { nested: 'x' }, ['x']]) {
      expect(kind(allowMatch, 'f', { cmd: v })).toBe('needs_approval');
    }
    expect(kind(allowMatch, 'f', { cmd: 'something' })).toBe('allow');
  });

  it('a non-object args payload (array / string / null) cannot satisfy a constraint', () => {
    for (const args of [['n', 0], 'n=0', null, 42]) {
      expect(kind(allowExactNumber, 'f', args)).toBe('needs_approval');
    }
  });

  it('a missing constrained field is a mismatch, not a wildcard', () => {
    expect(kind(allowExactNumber, 'f', { other: 0 })).toBe('needs_approval');
  });

  it('unknown functions are never auto-allowed', () => {
    expect(kind(allowMatch, 'shell::exec', { cmd: 'anything' })).toBe('needs_approval');
  });
});

describe('process integrity: hostile keys cannot corrupt evaluation', () => {
  it('a constraint field colliding with an inherited member does not spuriously match', () => {
    const yaml = `
version: 1
rules:
  - function: f
    action: allow
    args:
      toString:
        matches: ".*"
  - function: g
    action: allow
    args:
      constructor:
        equals: "x"
`;
    // obj['toString'] / obj['constructor'] resolve to prototype members, not
    // the (absent) own args — neither must produce an allow.
    expect(kind(yaml, 'f', {})).toBe('needs_approval');
    expect(kind(yaml, 'g', {})).toBe('needs_approval');
  });

  it('a __proto__ key in args evaluates normally and does not pollute Object.prototype', () => {
    const yaml = `
version: 1
rules:
  - function: f
    action: allow
    args:
      a:
        equals: 1
`;
    const evil = JSON.parse('{"a": 1, "__proto__": {"polluted": true}}');
    expect(p(yaml).check('f', evil).kind).toBe('allow');
    expect(({} as Record<string, unknown>).polluted).toBeUndefined();
  });
});

describe('allow fires only on the intended match', () => {
  it('bare-string rule allows the function and uses rule_id = function_id', () => {
    expect(p('version: 1\nrules:\n  - shell::fs::ls').check('shell::fs::ls', {})).toEqual({
      kind: 'allow',
      rule_id: 'shell::fs::ls',
    });
  });

  it('an unconstrained allow is args-agnostic (operators must add constraints to scope it)', () => {
    // Documents the contract: a bare allow grants the function for ANY args.
    expect(p('version: 1\nrules:\n  - f').check('f', ['anything', { x: 1 }]).kind).toBe('allow');
  });

  it('catch-all * allows any function_id when no earlier rule matches', () => {
    expect(p('version: 1\nrules:\n  - "*"').check('shell::exec', { command: 'rm -rf /' })).toEqual({
      kind: 'allow',
      rule_id: '*',
    });
  });

  it('catch-all * after a deny still denies the blocked function', () => {
    const yaml = `
version: 1
rules:
  - "!state::set"
  - "*"
`;
    expect(kind(yaml, 'state::set', {})).toBe('deny');
    expect(kind(yaml, 'shell::exec', { command: 'anything' })).toBe('allow');
  });

  it('catch-all * before denies allows everything (dev foot-gun)', () => {
    const yaml = `
version: 1
rules:
  - "*"
  - "!state::set"
`;
    expect(kind(yaml, 'state::set', {})).toBe('allow');
  });

  it('catch-all ::* matches only ids starting with ::', () => {
    const perms = p('version: 1\nrules:\n  - "::*"');
    expect(perms.check('::internal', {})).toEqual({ kind: 'allow', rule_id: '::*' });
    expect(perms.check('shell::exec', {})).toEqual({ kind: 'needs_approval' });
  });

  it('a namespace glob is anchored — prefix/suffix lookalikes do not slip into allow', () => {
    const yaml = `
version: 1
rules:
  - shell::*
`;
    // Intended matches: the namespace itself and anything nested under it.
    expect(kind(yaml, 'shell::exec', {})).toBe('allow');
    expect(kind(yaml, 'shell::fs::read', {})).toBe('allow');
    // Anchoring holds: a leading lookalike, a missing separator, or a bare
    // prefix must NOT satisfy `^shell::.*$`.
    for (const fid of ['Xshell::exec', 'shellexec', 'shell', 'not-shell::exec']) {
      expect(kind(yaml, fid, {})).toBe('needs_approval');
    }
  });

  it('a deny glob blocks every id under the namespace and cannot be dodged by appending levels', () => {
    const yaml = `
version: 1
rules:
  - "!shell::*"
  - "*"
`;
    // No suffix — deeper nesting, hostile args, anything — escapes the deny.
    for (const fid of ['shell::exec', 'shell::fs::read', 'shell::fs::sub::rm']) {
      expect(kind(yaml, fid, { command: 'rm -rf /' })).toBe('deny');
    }
    // A different namespace is governed by the catch-all, not the deny.
    expect(kind(yaml, 'state::get', {})).toBe('allow');
    // FOOTGUN: the deny is anchored, so a prefixed lookalike (`Xshell::exec`)
    // is NOT caught and falls through to the catch-all allow. A namespace deny
    // protects the exact prefix only.
    expect(kind(yaml, 'Xshell::exec', {})).toBe('allow');
  });

  it('a deny glob ignores args — it covers the namespace unconditionally', () => {
    const yaml = 'version: 1\nrules:\n  - "!shell::*"';
    for (const args of [{}, { command: 'ls' }, null, ['x'], JSON.parse('{"__proto__":{"x":1}}')]) {
      expect(p(yaml).check('shell::exec', args).kind).toBe('deny');
    }
  });

  it('mid- and suffix-position globs span separators but stay end-anchored', () => {
    const yaml = `
version: 1
rules:
  - shell::fs::*
  - '*::list'
`;
    // `*` spans `::`, so deep nesting under the prefix matches.
    expect(kind(yaml, 'shell::fs::read', {})).toBe('allow');
    expect(kind(yaml, 'shell::fs::sub::deep', {})).toBe('allow');
    // But the literal prefix is required verbatim — no separator, no match.
    expect(kind(yaml, 'shell::fsread', {})).toBe('needs_approval');
    expect(kind(yaml, 'shell::exec', {})).toBe('needs_approval');
    // `*::list` is broad on the left but anchored on the right: only ids that
    // END in `::list` match — any trailing suffix defeats it.
    expect(kind(yaml, 'models::list', {})).toBe('allow');
    expect(kind(yaml, 'evil::list', {})).toBe('allow');
    expect(kind(yaml, 'models::listing', {})).toBe('needs_approval');
    expect(kind(yaml, 'models::list::sub', {})).toBe('needs_approval');
    expect(kind(yaml, 'models::get', {})).toBe('needs_approval');
  });

  it('only * is a wildcard — regex metacharacters in a pattern match literally', () => {
    // If `.` leaked through as regex "any char", this allow would over-match
    // (aXc::ping) and a symmetric deny could be evaded. Escaping must hold.
    const yaml = "version: 1\nrules:\n  - 'a.c::*'";
    expect(kind(yaml, 'a.c::ping', {})).toBe('allow');
    expect(kind(yaml, 'aXc::ping', {})).toBe('needs_approval');
  });

  it('a newline in the function_id cannot tunnel a deny glob into a later allow', () => {
    // The compiled regex is line-unaware (`.` does not cross `\n`), so an
    // embedded newline makes BOTH the deny glob and the catch-all fail to
    // match — the call fails closed to needs_approval instead of slipping
    // through to the broad allow.
    const yaml = `
version: 1
rules:
  - "!shell::*"
  - "*"
`;
    expect(kind(yaml, 'shell::exec\nrm -rf /', {})).toBe('needs_approval');
  });

  it('a constrained allow on an exact function still works — and composes with a glob', () => {
    // Glob support is additive: an exact function id compiles to glob=null and
    // matches by ===, with arg constraints AND-checked exactly as before.
    const exact = `
version: 1
rules:
  - function: shell::fs::write
    action: allow
    args:
      path:
        matches: '^/tmp/'
`;
    expect(kind(exact, 'shell::fs::write', { path: '/tmp/cache' })).toBe('allow');
    expect(kind(exact, 'shell::fs::write', { path: '/etc/passwd' })).toBe('needs_approval');
    // The SAME arg constraints compose with a globbed function id.
    const globbed = `
version: 1
rules:
  - function: shell::fs::*
    action: allow
    args:
      path:
        matches: '^/tmp/'
`;
    expect(kind(globbed, 'shell::fs::write', { path: '/tmp/x' })).toBe('allow');
    expect(kind(globbed, 'shell::fs::read', { path: '/tmp/x' })).toBe('allow');
    expect(kind(globbed, 'shell::fs::write', { path: '/root/.ssh' })).toBe('needs_approval');
  });

  it('AND-combines multiple constraints — one miss is a full mismatch', () => {
    const yaml = `
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
`;
    expect(kind(yaml, 'state::set', { scope: 'agent', key: 'session/abc/notes' })).toBe('allow');
    expect(kind(yaml, 'state::set', { scope: 'global', key: 'session/abc/notes' })).toBe(
      'needs_approval',
    );
    expect(kind(yaml, 'state::set', { scope: 'agent', key: '../../secrets' })).toBe(
      'needs_approval',
    );
  });

  it('a ( |$) word-boundary guard blocks separators and newlines — but not a space-suffixed payload', () => {
    const yaml = `
version: 1
rules:
  - function: shell::exec
    action: allow
    args:
      command:
        matches: "^git (status|log|diff)( |$)"
`;
    // Resisted: anything that breaks the word boundary or moves off the start.
    for (const command of [
      'git status; rm -rf /',
      'git status\nrm -rf /',
      'git statuses',
      'X git status',
    ]) {
      expect(kind(yaml, 'shell::exec', { command })).toBe('needs_approval');
    }
    // Allowed — including the residual hole: because the pattern is not
    // end-anchored, a single space opens the door to an arbitrary suffix.
    for (const command of ['git status', 'git log ', 'git status && curl evil']) {
      expect(kind(yaml, 'shell::exec', { command })).toBe('allow');
    }
  });

  it('FOOTGUN: the engine search-matches a prefix — only an end-anchor ($) closes the suffix hole', () => {
    // The engine runs `regex.test()` — a search anchored only where the
    // pattern says, not a full-string match. A start-anchored pattern permits
    // arbitrary suffixes (command chaining); adding `$` is what makes it safe.
    // This is rule-author responsibility; the assertions document the
    // dangerous-but-faithful behaviour so any future anchoring change surfaces.
    const mk = (pat: string) => `
version: 1
rules:
  - function: shell::exec
    action: allow
    args:
      command:
        matches: ${JSON.stringify(pat)}
`;
    const leaky = mk('^git log');
    const anchored = mk('^git (status|log|diff)$');

    expect(kind(leaky, 'shell::exec', { command: 'git log; rm -rf /' })).toBe('allow');
    expect(kind(anchored, 'shell::exec', { command: 'git status && curl evil' })).toBe(
      'needs_approval',
    );
    expect(kind(anchored, 'shell::exec', { command: 'git status ' })).toBe('needs_approval');
    expect(kind(anchored, 'shell::exec', { command: 'git status' })).toBe('allow');
  });
});

describe('deny precedence and evasion', () => {
  it('"!fn" shorthand denies with rule_id = function_id', () => {
    const d = p('version: 1\nrules:\n  - "!approval::resolve"').check('approval::resolve', {});
    expect(d.kind).toBe('deny');
    if (d.kind === 'deny') expect(d.rule_id).toBe('approval::resolve');
  });

  it('a detailed deny defaults rule_id to the function and reports matched_constraint', () => {
    const yaml = `
version: 1
rules:
  - function: shell::exec
    action: deny
    args:
      command:
        matches: "^git push --force"
  - rule_id: git/all
    function: shell::exec
    action: allow
`;
    const d = p(yaml).check('shell::exec', { command: 'git push --force origin main' });
    expect(d.kind).toBe('deny');
    if (d.kind === 'deny') {
      expect(d.rule_id).toBe('shell::exec');
      expect(d.matched_constraint).toEqual({
        field: 'command',
        operator: 'matches',
        value: '^git push --force',
      });
    }
    // The deny only covers what its constraint covers; benign calls reach allow.
    expect(kind(yaml, 'shell::exec', { command: 'git status' })).toBe('allow');
  });

  it('an unconstrained deny before a broad allow blocks every variant of that function', () => {
    const yaml = `
version: 1
rules:
  - "!shell::exec"
  - shell::exec
`;
    for (const command of ['git status', 'rm -rf /', '', '   ']) {
      expect(kind(yaml, 'shell::exec', { command })).toBe('deny');
    }
  });

  it('FOOTGUN: a dodgeable constrained deny before a broad allow is bypassable', () => {
    // If a deny constraint can be side-stepped (here: a leading space defeats
    // "^rm "), a later broad allow catches the call — so denies must be
    // unconstrained and first, exactly as the shipped file is written.
    const yaml = `
version: 1
rules:
  - function: shell::exec
    action: deny
    args:
      command:
        matches: "^rm "
  - shell::exec
`;
    expect(kind(yaml, 'shell::exec', { command: 'rm -rf /' })).toBe('deny');
    expect(kind(yaml, 'shell::exec', { command: ' rm -rf /' })).toBe('allow');
  });

  it('first-match-wins: an earlier scoped allow beats a later blanket deny', () => {
    const yaml = `
version: 1
rules:
  - rule_id: git/status-only
    function: shell::exec
    action: allow
    args:
      command:
        equals: "git status"
  - "!shell::exec"
`;
    expect(kind(yaml, 'shell::exec', { command: 'git status' })).toBe('allow');
    const d = p(yaml).check('shell::exec', { command: 'git diff' });
    expect(d.kind).toBe('deny');
  });
});

describe('decisionToValue — wire mapping is closed and lossless', () => {
  it('maps allow', () => {
    expect(decisionToValue({ kind: 'allow', rule_id: 'r' })).toEqual({
      decision: 'allow',
      rule_id: 'r',
    });
  });

  it('includes matched_constraint on deny only when present', () => {
    const withConstraint = decisionToValue({
      kind: 'deny',
      rule_id: 'r',
      matched_constraint: { field: 'command', operator: 'matches', value: '^x' },
    });
    expect(withConstraint).toEqual({
      decision: 'deny',
      rule_id: 'r',
      rule_action: 'deny',
      matched_constraint: { field: 'command', operator: 'matches', value: '^x' },
    });

    const noConstraint = decisionToValue({ kind: 'deny', rule_id: 'r', matched_constraint: null });
    expect(noConstraint).toEqual({ decision: 'deny', rule_id: 'r', rule_action: 'deny' });
    expect('matched_constraint' in noConstraint).toBe(false);
  });

  it('maps needs_approval, and an unrecognised kind defaults to needs_approval', () => {
    expect(decisionToValue({ kind: 'needs_approval' })).toEqual({ decision: 'needs_approval' });
    expect(decisionToValue({ kind: 'sneaky-allow' } as unknown as Decision)).toEqual({
      decision: 'needs_approval',
    });
  });
});

describe('policy::check_permissions handler — ingress validation', () => {
  function captureHandler(perms: Permissions) {
    let handler: ((input: unknown) => Promise<unknown>) | undefined;
    const iii = {
      registerFunction: (_id: string, h: (input: unknown) => Promise<unknown>) => {
        handler = h;
      },
    };
    registerPolicy(iii as never, { current: () => perms } as unknown as PermissionsHandle);
    if (!handler) throw new Error('handler not registered');
    return handler;
  }

  it('evaluates a well-formed request end-to-end', async () => {
    const handler = captureHandler(p('version: 1\nrules:\n  - "!state::set"'));
    await expect(handler({ function_id: 'state::set' })).resolves.toEqual({
      decision: 'deny',
      rule_id: 'state::set',
      rule_action: 'deny',
    });
    await expect(handler({ function_id: 'unknown::fn' })).resolves.toEqual({
      decision: 'needs_approval',
    });
  });

  it('rejects a request with a missing or empty function_id', async () => {
    const handler = captureHandler(Permissions.empty());
    await expect(handler({})).rejects.toThrow();
    await expect(handler({ function_id: '' })).rejects.toThrow();
  });
});

describe('shipped iii-permissions.yaml', () => {
  let shipped: Permissions;
  const load = async () => {
    if (!shipped) {
      const { readFile } = await import('node:fs/promises');
      shipped = Permissions.parse(await readFile('./iii-permissions.yaml', 'utf8'));
    }
    return shipped;
  };

  const DENIED = [
    'approval::resolve',
    'policy::check_permissions',
    'hook-fanout::publish_collect',
    'state::set',
    'state::update',
    'state::delete',
    'stream::set',
    'iii::durable::publish',
    'harness::provider::resolve',
    'harness::provider::register',
    'configuration::get',
    'configuration::set',
    'configuration::register',
    'run::start',
    'router::stream_assistant',
  ];

  it('kernel surfaces are denied unconditionally — hostile args cannot dodge them', async () => {
    const perms = await load();
    const hostileArgs = [
      {},
      { command: 'anything' },
      { scope: 'agent', key: 'session/a/notes' },
      JSON.parse('{"__proto__": {"x": 1}}'),
    ];
    for (const fid of DENIED) {
      for (const args of hostileArgs) {
        const d = perms.check(fid, args);
        expect(d.kind).toBe('deny'); // never allow, never needs_approval
        if (d.kind === 'deny') expect(d.rule_id).toBe(fid);
      }
    }
  });

  it('allows the read-only conveniences', async () => {
    const perms = await load();
    for (const fid of ['state::get', 'state::list', 'models::list', 'models::get']) {
      expect(perms.check(fid, {}).kind).toBe('allow');
    }
  });

  it('a dangerous function absent from the file is gated, not allowed', async () => {
    const perms = await load();
    expect(perms.check('shell::exec', { command: 'rm -rf /' }).kind).toBe('needs_approval');
  });
});

describe('PermissionsHandle — a broken policy file must fail closed', () => {
  let dir: string;
  const tmp = async (contents: string) => {
    if (!dir) dir = await mkdtemp(join(tmpdir(), 'policy-test-'));
    const file = join(dir, `perms-${Math.random().toString(36).slice(2)}.yaml`);
    await writeFile(file, contents, 'utf8');
    return file;
  };

  afterAll(async () => {
    if (dir) await rm(dir, { recursive: true, force: true });
  });

  it('a missing file loads empty permissions (everything needs approval)', async () => {
    const handle = new RealPermissionsHandle(join(tmpdir(), 'does-not-exist-xyz.yaml'));
    await handle.load();
    expect(handle.current().ruleCount()).toBe(0);
    expect(handle.current().check('state::set', {}).kind).toBe('needs_approval');
  });

  it('a file with a malformed rule loads empty rather than throwing or fail-opening', async () => {
    const file = await tmp(
      'version: 1\nrules:\n  - function: f\n    action: definitely-not-valid\n',
    );
    const handle = new RealPermissionsHandle(file);
    await expect(handle.load()).resolves.toBeUndefined();
    expect(handle.current().ruleCount()).toBe(0);
  });

  it('a valid file loads its rules and enforces them', async () => {
    const file = await tmp('version: 1\nrules:\n  - "!state::set"\n  - state::get\n');
    const handle = new RealPermissionsHandle(file);
    await handle.load();
    expect(handle.current().ruleCount()).toBe(2);
    expect(handle.current().check('state::set', {}).kind).toBe('deny');
    expect(handle.current().check('state::get', {}).kind).toBe('allow');
  });
});
