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
    expect(out).toContain('agent_call');
    expect(out).toContain('directory::skills::get');
  });

  it('skills appear in config order', () => {
    const out = buildSystemPrompt(
      [defaultSkillBody('iii://iii', 'AAA'), defaultSkillBody('iii://shell', 'BBB')],
      null,
    );
    expect(out.indexOf('AAA')).toBeLessThan(out.indexOf('BBB'));
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
