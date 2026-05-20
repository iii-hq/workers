import { describe, expect, it } from 'vitest';
import { buildSystemPrompt, defaultSkillBody } from '../../src/turn-orchestrator/system-prompt.js';

describe('buildSystemPrompt', () => {
  it('non-empty override returns verbatim', () => {
    expect(buildSystemPrompt([defaultSkillBody('iii://iii', 'body')], '/tmp', 'custom')).toBe(
      'custom',
    );
  });

  it('empty override falls through to canonical assembly', () => {
    const out = buildSystemPrompt([defaultSkillBody('iii://iii', 'BODY')], '/tmp', '');
    expect(out).toContain('You are an iii agent worker');
    expect(out).toContain('/tmp');
    expect(out).toContain('BODY');
  });

  it('failed skill produces recovery stub with bare id', () => {
    const out = buildSystemPrompt([defaultSkillBody('iii://iii', null)], null);
    expect(out).toContain('# iii://iii');
    expect(out).toContain('directory::skills::get { id: "iii" }');
  });

  it('preamble identity preserved', () => {
    const out = buildSystemPrompt([], null);
    expect(out).toContain('You are an iii agent worker.');
    expect(out).toContain('agent_trigger');
    expect(out).toContain('directory::skills::get');
  });

  it('preamble teaches the @fn(<id>) pill syntax', () => {
    const out = buildSystemPrompt([], null);
    expect(out).toContain('@fn(<function_id>)');
    expect(out).toContain('@fn(directory::skills::get)');
  });

  it('skills appear in config order', () => {
    const out = buildSystemPrompt(
      [defaultSkillBody('iii://iii', 'AAA'), defaultSkillBody('iii://shell', 'BBB')],
      null,
    );
    expect(out.indexOf('AAA')).toBeLessThan(out.indexOf('BBB'));
  });

  it('mode plan prepends planner paragraph before identity preamble', () => {
    const out = buildSystemPrompt([], null, null, 'plan');
    expect(out).toContain('operating in plan mode');
    expect(out.indexOf('operating in plan mode')).toBeLessThan(
      out.indexOf('You are an iii agent worker'),
    );
  });

  it('mode ask prepends ask paragraph before identity preamble', () => {
    const out = buildSystemPrompt([], null, null, 'ask');
    expect(out).toContain('operating in ask mode');
    expect(out.indexOf('operating in ask mode')).toBeLessThan(
      out.indexOf('You are an iii agent worker'),
    );
  });

  it('mode agent prepends agent paragraph before identity preamble', () => {
    const out = buildSystemPrompt([], null, null, 'agent');
    expect(out).toContain('operating in agent mode');
    expect(out.indexOf('operating in agent mode')).toBeLessThan(
      out.indexOf('You are an iii agent worker'),
    );
  });

  it('omitting mode preserves the canonical preamble verbatim (no mode paragraph)', () => {
    const out = buildSystemPrompt([], null);
    expect(out.startsWith('You are an iii agent worker')).toBe(true);
    expect(out).not.toContain('operating in plan mode');
    expect(out).not.toContain('operating in ask mode');
    expect(out).not.toContain('operating in agent mode');
  });

  it('mode null behaves like omitted (backwards compat for non-console callers)', () => {
    const out = buildSystemPrompt([], null, null, null);
    expect(out.startsWith('You are an iii agent worker')).toBe(true);
    expect(out).not.toContain('operating in');
  });

  it('non-empty override wins over mode (override returned verbatim)', () => {
    const out = buildSystemPrompt([], '/tmp', 'custom-override', 'plan');
    expect(out).toBe('custom-override');
  });

  it('mode interacts with cwd and skills: paragraph, preamble, cwd, skill body in order', () => {
    const out = buildSystemPrompt(
      [defaultSkillBody('iii://iii', 'SKILLBODY')],
      '/work',
      null,
      'agent',
    );
    const pAgent = out.indexOf('operating in agent mode');
    const pIdentity = out.indexOf('You are an iii agent worker');
    const pCwd = out.indexOf('/work');
    const pSkill = out.indexOf('SKILLBODY');
    expect(pAgent).toBeLessThan(pIdentity);
    expect(pIdentity).toBeLessThan(pCwd);
    expect(pCwd).toBeLessThan(pSkill);
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
