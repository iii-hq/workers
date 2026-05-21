// "Tool Calls Made" section retains tool-call awareness even though prune
// strips the actual outputs before summarisation.
export const SUMMARY_TEMPLATE = `Output exactly the Markdown structure shown inside <template> and keep the section order unchanged. Do not include the <template> tags in your response.
<template>
## Goal
- [single-sentence task summary]

## Constraints & Preferences
- [user constraints, preferences, specs, or "(none)"]

## Progress
### Done
- [completed work or "(none)"]

### In Progress
- [current work or "(none)"]

### Blocked
- [blockers or "(none)"]

## Key Decisions
- [decision and why, or "(none)"]

## Tool Calls Made
- [notable tool calls with their arguments (terse), or "(none)"]

## Next Steps
- [ordered next actions or "(none)"]

## Critical Context
- [important technical facts, errors, open questions, or "(none)"]

## Relevant Files
- [file or directory path: why it matters, or "(none)"]
</template>

Rules:
- Keep every section, even when empty.
- Use terse bullets, not prose paragraphs.
- Preserve exact file paths, commands, error strings, and identifiers when known.
- Do not mention the summary process or that context was compacted.`;

export function buildPrompt(input: {
  previousSummary?: string;
  context: string[];
}): string {
  const anchor = input.previousSummary
    ? [
        'Update the anchored summary below using the conversation history above.',
        'Preserve still-true details, remove stale details, and merge in the new facts.',
        '<previous-summary>',
        input.previousSummary,
        '</previous-summary>',
      ].join('\n')
    : 'Create a new anchored summary from the conversation history above.';
  return [anchor, SUMMARY_TEMPLATE, ...input.context].join('\n\n');
}
