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

  it('preamble instructs fetching per-function skill before first call', () => {
    // Regression: kimi-k2.6 (and other LLMs) often jump from the worker
    // index straight to a function call, guess field names, and burn
    // turns on retries. The preamble must explicitly tell them to fetch
    // the per-function skill body first.
    const out = buildSystemPrompt([]);
    expect(out).toContain('FIRST time');
    expect(out).toContain('<worker>/<function>');
    expect(out).toContain('sandbox/exec');
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
