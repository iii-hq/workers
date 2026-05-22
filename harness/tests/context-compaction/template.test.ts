import { describe, expect, it } from 'vitest';
import { SUMMARY_TEMPLATE, buildPrompt } from '../../src/context-compaction/template.js';

describe('SUMMARY_TEMPLATE', () => {
  it('contains the harness-specific Tool Calls Made section', () => {
    expect(SUMMARY_TEMPLATE).toContain('## Tool Calls Made');
  });
  it('contains all required sections', () => {
    for (const h of [
      '## Goal',
      '## Constraints & Preferences',
      '## Progress',
      '### Done',
      '### In Progress',
      '### Blocked',
      '## Key Decisions',
      '## Next Steps',
      '## Critical Context',
      '## Relevant Files',
    ]) {
      expect(SUMMARY_TEMPLATE).toContain(h);
    }
  });
});

describe('buildPrompt', () => {
  it('omits previous-summary block on first compaction', () => {
    const p = buildPrompt({ previousSummary: undefined, context: [] });
    expect(p).not.toContain('<previous-summary>');
    expect(p).toContain('Create a new anchored summary');
  });
  it('injects previous-summary block when present', () => {
    const p = buildPrompt({ previousSummary: 'prior text', context: [] });
    expect(p).toContain('<previous-summary>');
    expect(p).toContain('prior text');
    expect(p).toContain('Update the anchored summary');
  });
  it('appends extra context blocks', () => {
    const p = buildPrompt({ previousSummary: undefined, context: ['extra-note'] });
    expect(p).toContain('extra-note');
  });
});
