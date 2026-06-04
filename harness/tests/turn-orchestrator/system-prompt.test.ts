import { describe, expect, it } from 'vitest';
import {
  buildSystemPrompt,
  defaultSkillBody,
  skillIdFromUri,
} from '../../src/turn-orchestrator/system-prompt.js';

describe('buildSystemPrompt', () => {
  it('non-empty override returns verbatim', () => {
    expect(buildSystemPrompt([defaultSkillBody('iii://iii', 'body')], { override: 'custom' })).toBe(
      'custom',
    );
  });

  it('empty override falls through to canonical assembly', () => {
    const out = buildSystemPrompt([defaultSkillBody('iii://iii', 'BODY')], { override: '' });
    expect(out).toContain('You are an iii agent worker');
    expect(out).toContain('BODY');
  });

  it('failed skill produces recovery stub with bare id', () => {
    const out = buildSystemPrompt([defaultSkillBody('iii://iii', null)]);
    expect(out).toContain('# iii://iii');
    expect(out).toContain('directory::skills::get { id: "iii" }');
  });

  it('preamble identity preserved', () => {
    const out = buildSystemPrompt([]);
    expect(out).toContain('You are an iii agent worker.');
    expect(out).toContain('agent_trigger');
    expect(out).toContain('directory::skills::get');
  });

  it('preamble teaches the @fn(<id>) pill syntax', () => {
    const out = buildSystemPrompt([]);
    expect(out).toContain('@fn(<function_id>)');
    expect(out).toContain('@fn(directory::skills::get)');
  });

  it('preamble mandates checking the contract via engine::functions::info before any call (H6)', () => {
    // Regression: LLMs jump straight to a function call and guess field
    // names, burning turns on retries. The preamble must (a) make
    // engine::functions::info the mandatory contract check before ANY call,
    // and (b) steer skill reads to the worker id, since the old
    // `<worker>/<function>` fetch usually 404s.
    const out = buildSystemPrompt([]);
    expect(out).toContain('BEFORE you call ANY function');
    expect(out).toContain('engine::functions::info');
    expect(out).toContain('THIS IS THE API CONTRACT.');
    // Generic anti-pattern id (no worker-specific example).
    expect(out).toContain('<worker>/<function>');
  });

  it('preamble marks function_id as REQUIRED on engine::functions::info with a concrete example', () => {
    // Root cause observed live: the model passed `engine::functions::info` AS
    // the function_id (introspecting the discovery tool itself), which returns
    // that function's own metadata. The preamble must require function_id, show
    // a concrete TARGET value, forbid passing a discovery call as the id, and
    // describe the real missing-field error (NOT "self-metadata on omit").
    const out = buildSystemPrompt([]);
    expect(out).toContain('`function_id` argument is REQUIRED');
    expect(out).toContain('{ function_id: "shell::fs::ls" }');
    // Self-introspection trap: passing the info call's own id returns info-about-info.
    expect(out).toMatch(/metadata ABOUT the info function/);
    expect(out).toMatch(/never introspect them/);
    // Accurate omit behavior is a hard error, not self-metadata.
    expect(out).toMatch(/missing field/);
    // Contrast: list is the no-id browse call.
    expect(out).toContain('takes NO id');
  });

  it('preamble forbids serializing payload as a string, with a WRONG/RIGHT example (haiku stringify trap)', () => {
    // Root cause observed live (haiku, todo-app build): the model put the whole
    // payload as a JSON-ENCODED STRING into `payload`, so the worker rejected it
    // with `serialization error: invalid type: string ..., expected struct`. The
    // preamble must state payload is an object literal, call out the long/multi-
    // line-value case (the trigger for stringifying source code), and show the
    // exact wrong-vs-right shape.
    const out = buildSystemPrompt([]);
    expect(out).toContain('`payload` is a JSON OBJECT, never a string');
    expect(out).toMatch(/expected struct/);
    expect(out).toContain('WRONG');
    expect(out).toContain('RIGHT');
    // Long/multi-line values must still live as a string VALUE of a field.
    expect(out).toMatch(/long or multi-line/);
  });

  it('preamble teaches error-driven correction and forbids blind identical retries', () => {
    // Root cause observed live: the model re-sent the same failing call (10
    // timeouts, repeated VM-boot failures) without changing anything, and
    // mis-read `invalid_arguments` as an infra problem. The preamble must map
    // each error class to a corrective action and ban resending an unchanged
    // failed call.
    const out = buildSystemPrompt([]);
    expect(out).toContain('never resend the same `function` + `payload` unchanged');
    expect(out).toContain('Resending an identical failed call is never the fix.');
    // invalid_arguments must be read as a caller payload error, not infra.
    expect(out).toMatch(/`invalid_arguments`[\s\S]*YOUR payload is wrong/);
    // A repeating timeout/infra error must stop the retry loop.
    expect(out).toMatch(/timeout or an infrastructure\/transport error that REPEATS/);
  });

  it('preamble distinguishes a list description (hint) from the info contract', () => {
    // The model had `engine::functions::list` descriptions yet still guessed
    // payloads. The preamble must say the list one-liner is a hint and `info` is
    // the authoritative contract.
    const out = buildSystemPrompt([]);
    expect(out).toMatch(/is a HINT, not the contract/);
  });

  it('preamble encodes the 3-tier discovery hierarchy: engine, directory(skills), registry', () => {
    // Engine (the iii instance) gives the per-call CONTRACT; the directory gives
    // the APPROACH (skills, loaded before building); the registry is only for
    // adding a NEW worker.
    const out = buildSystemPrompt([]);
    expect(out).toContain('THE ENGINE (the iii instance)');
    expect(out).toContain('THE DIRECTORY (skills)');
    expect(out).toMatch(/THE REGISTRY — ONLY to ADD A NEW worker/);
    // Registry path ends in an install, not a lookup.
    expect(out).toContain('worker::add');
  });

  it('preamble treats skills as load-before-building, NOT a last resort (mqvc4qtb: agent skipped the worker skill and imported express)', () => {
    // Root cause observed live: the old "skills ONLY when the engine schema is
    // not enough" framing led the agent to never load the runtime worker's skill
    // and import a foreign-ecosystem pattern. The preamble must (a) frame skills
    // as the APPROACH to load before building, not a fallback, and (b) split
    // engine=contract vs skill=approach.
    const out = buildSystemPrompt([]);
    expect(out).toMatch(/LOAD IT BEFORE you build/);
    expect(out).not.toMatch(/ONLY when the engine schema is not enough/);
    expect(out).toMatch(
      /the engine \(source 1\) gives the exact per-call CONTRACT; the skill gives\s+the APPROACH/,
    );
  });

  it('preamble forbids importing foreign-ecosystem patterns before reading the worker skill (build-first directive)', () => {
    const out = buildSystemPrompt([]);
    expect(out).toMatch(/When the task is to BUILD, AUTHOR, or OPERATE/);
    // Must tell the agent to load EVERY involved worker's skill, incl. the runtime.
    expect(out).toMatch(/identify EVERY worker the task\s+touches/);
    expect(out).toMatch(/load each one's skill with `directory::skills::get`/);
    // Must block carrying non-iii patterns in from memory.
    expect(out).toMatch(/Do NOT carry patterns from other ecosystems/);
    expect(out).toMatch(/not an iii function, stop and read the relevant worker's skill/);
  });

  it('preamble documents directory::skills::index for discovering available skills', () => {
    const out = buildSystemPrompt([]);
    expect(out).toContain('directory::skills::index');
    expect(out).toMatch(/WHAT skills exist/);
  });

  it('preamble is generic — no worker-specific examples leak into the identity prompt', () => {
    const out = buildSystemPrompt([]);
    expect(out.toLowerCase()).not.toContain('sandbox');
    expect(out).not.toContain('heredoc');
  });

  it('preamble enumerates the discovery surface (H6)', () => {
    const out = buildSystemPrompt([]);
    for (const fn of [
      'engine::functions::list',
      'engine::workers::list',
      'engine::triggers::list',
      'engine::registered-triggers::list',
      'worker::list',
      'directory::registry::workers::list',
      'directory::skills::index',
      'directory::skills::get',
    ]) {
      expect(out).toContain(fn);
    }
  });

  it('preamble checks a RUNNING worker via engine::workers::list + worker::list, not the registry', () => {
    const out = buildSystemPrompt([]);
    expect(out).toMatch(/To check a worker is RUNNING/i);
    expect(out).toContain('engine::workers::list');
    expect(out).toContain('worker::list');
    // Must steer AWAY from the registry list for liveness checks.
    expect(out).toMatch(/never `directory::registry::workers::list`/);
  });

  it('preamble carries the worker-authoring entry point (H4)', () => {
    // registerFunction/registerTrigger are methods on registerWorker()'s
    // return value, NOT top-level exports — the #1 worker-authoring footgun.
    const out = buildSystemPrompt([]);
    expect(out).toContain('registerWorker');
    expect(out).toContain('iii.registerFunction');
    // The error phrase wraps across a line in the preamble template literal.
    expect(out).toMatch(/TypeError: registerFunction is not a\s+function/);
  });

  it('skills appear in config order', () => {
    const out = buildSystemPrompt([
      defaultSkillBody('iii://iii', 'AAA'),
      defaultSkillBody('iii://shell', 'BBB'),
    ]);
    expect(out.indexOf('AAA')).toBeLessThan(out.indexOf('BBB'));
  });

  it('mode plan prepends planner paragraph before identity preamble', () => {
    const out = buildSystemPrompt([], { mode: 'plan' });
    expect(out).toContain('operating in plan mode');
    expect(out.indexOf('operating in plan mode')).toBeLessThan(
      out.indexOf('You are an iii agent worker'),
    );
  });

  it('mode ask prepends ask paragraph before identity preamble', () => {
    const out = buildSystemPrompt([], { mode: 'ask' });
    expect(out).toContain('operating in ask mode');
    expect(out.indexOf('operating in ask mode')).toBeLessThan(
      out.indexOf('You are an iii agent worker'),
    );
  });

  it('mode agent prepends agent paragraph before identity preamble', () => {
    const out = buildSystemPrompt([], { mode: 'agent' });
    expect(out).toContain('operating in agent mode');
    expect(out.indexOf('operating in agent mode')).toBeLessThan(
      out.indexOf('You are an iii agent worker'),
    );
  });

  it('omitting mode preserves the canonical preamble verbatim (no mode paragraph)', () => {
    const out = buildSystemPrompt([]);
    expect(out.startsWith('You are an iii agent worker')).toBe(true);
    expect(out).not.toContain('operating in plan mode');
    expect(out).not.toContain('operating in ask mode');
    expect(out).not.toContain('operating in agent mode');
  });

  it('mode null behaves like omitted (backwards compat for non-console callers)', () => {
    const out = buildSystemPrompt([], { mode: null });
    expect(out.startsWith('You are an iii agent worker')).toBe(true);
    expect(out).not.toContain('operating in');
  });

  it('non-empty override wins over mode (override returned verbatim)', () => {
    const out = buildSystemPrompt([], { override: 'custom-override', mode: 'plan' });
    expect(out).toBe('custom-override');
  });

  it('mode interacts with skills: paragraph, preamble, skill body in order', () => {
    const out = buildSystemPrompt([defaultSkillBody('iii://iii', 'SKILLBODY')], { mode: 'agent' });
    const pAgent = out.indexOf('operating in agent mode');
    const pIdentity = out.indexOf('You are an iii agent worker');
    const pSkill = out.indexOf('SKILLBODY');
    expect(pAgent).toBeLessThan(pIdentity);
    expect(pIdentity).toBeLessThan(pSkill);
  });
});

describe('defaultSkillBody', () => {
  it('strips iii:// prefix to produce id', () => {
    const s = defaultSkillBody('iii://iii', null);
    expect(s.id).toBe('iii');
    expect(s.uri).toBe('iii://iii');
  });

  it('passes bare ids through unchanged', () => {
    const s = defaultSkillBody('iii', 'B');
    expect(s.id).toBe('iii');
  });
});

describe('skillIdFromUri', () => {
  it('strips the iii:// scheme and passes bare ids through', () => {
    expect(skillIdFromUri('iii://iii-directory/index')).toBe('iii-directory/index');
    expect(skillIdFromUri('iii-directory/index')).toBe('iii-directory/index');
  });
});
