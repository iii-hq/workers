/**
 * The workspace memory pi reads on startup. pi discovers `AGENTS.md` in its
 * working directory; the worker owns one marked block inside it and rewrites
 * that block on every boot, so anything the operator adds outside the markers
 * survives.
 *
 * What the block says about iii is NOT written here. The rules for working
 * against a live engine, and the index of installed skills, come from the
 * `iii-directory` worker (see `../iii-context.ts`) — one copy, one owner, and
 * the same text a headless turn is given. This module contributes only what the
 * directory cannot know: which workspace this is, which engine it talks to, and
 * which two workers have to stay up for the terminal to keep existing.
 */

export const NOTES_BEGIN =
  '<!-- iii:begin — written by the pi worker on every boot; edits inside are lost -->';
export const NOTES_END = '<!-- iii:end -->';

export type NotesOptions = {
  workspace: string;
  engineUrl: string;
  /** The iii context from `iii-directory`. Empty when it served nothing. */
  context: string;
  /** Why it is missing, when it is — the agent should be told, not left guessing. */
  detail?: string;
};

export function engineNotes(options: NotesOptions): string {
  const context = options.context.trim()
    ? options.context.trim()
    : `_The iii context is unavailable: ${
        options.detail || 'the `iii-directory` worker did not answer'
      }. Discover everything from the engine itself with \`iii trigger engine::functions::list\`, and tell the operator the directory is missing._`;

  return `# You run inside an iii engine

This workspace belongs to the \`pi\` iii worker: a terminal page on the iii
console runs you, and the \`shell\` worker owns the session. You have direct
access to the running engine and can create functions, triggers, and workers
that outlive this terminal.

- Engine WebSocket address: \`${options.engineUrl}\` (also in \`$III_URL\`).
- This workspace: \`${options.workspace}\` — the directory this terminal starts in.
- **Never stop or reconfigure \`pi\` or \`shell\`** — the first is this terminal,
  the second runs it. Restarting either kills the session you are typing in,
  mid-command.
- Your work is on the record: \`.pi/extensions/iii-activity.ts\` reports every
  prompt you answer and every tool you run onto \`agent::events\`, which is how
  the console shows this terminal's turns. Leave that extension in place.

${context}
`;
}
