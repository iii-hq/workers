/**
 * The workspace memory pi reads on startup. pi discovers `AGENTS.md` in its
 * working directory; the worker owns one marked block inside it and rewrites
 * that block on every boot, so anything the operator adds outside the markers
 * survives.
 */

export const NOTES_BEGIN =
  '<!-- iii:begin — written by the pi worker on every boot; edits inside are lost -->';
export const NOTES_END = '<!-- iii:end -->';

export function engineNotes(options: { workspace: string; engineUrl: string }): string {
  return `# You run inside an iii engine

This workspace belongs to the \`pi\` iii worker: a terminal page on the iii
console runs you, and the \`shell\` worker owns the session. You have direct
access to the running engine and can create functions, triggers, and workers
that outlive this terminal.

## The engine

- WebSocket address: \`${options.engineUrl}\` (also in \`$III_URL\`).
- \`iii trigger <function_id> key=value\` calls any function; add \`--help\` to a
  function to read its description and request schema off the running engine.
- \`iii trigger engine::functions::list\` is the catalogue of what already exists.

## Your workspace

\`${options.workspace}\` — the directory this terminal starts in. The iii skills
are installed here (\`.agents/skills\`, which is one of the places you look):
read \`iii-getting-started\`, \`iii-core-primitives\`, and \`iii-sdk-reference\`
before you build, and verify what they say against the running engine.

## Creating a function or trigger

Any process with an iii SDK is a worker. Scaffold a directory here, install
\`iii-sdk\`, and run it:

\`\`\`js
import { registerWorker } from 'iii-sdk';

const iii = registerWorker(process.env.III_URL ?? 'ws://127.0.0.1:49134', {
  workerName: 'my-worker',
});

iii.registerFunction('my-worker::greet', async ({ name }) => ({
  message: \`Hello, \${name}!\`,
}));

iii.registerTrigger({
  type: 'http',
  function_id: 'my-worker::greet',
  config: { api_path: '/greet', http_method: 'GET' },
});
\`\`\`

Function ids are \`<worker>::<verb>\`, kebab-case for multi-word segments.

## Rules

- **Never take a name that is already in use.** A worker name is the first half
  of every function id it registers, so a repeat collides with the other
  worker's ids. Check \`engine::functions::list\` first and pick something
  specific.
- **Never stop or reconfigure \`pi\` or \`shell\`** — the first is this
  terminal, the second runs it. Restarting either kills the session you are
  typing in, mid-command.
- **Ask how workers are installed here before you install one.** A registry
  worker is \`iii worker add <name>\` on some engines and a declared container in
  a compose project on others; the operator knows which, and the wrong one
  either does nothing or restarts everything.

## Your work is on the record

The \`.pi/extensions/iii-activity.ts\` extension reports every prompt you answer
and every tool you run to the engine, which streams them onto
\`agent::events\` — so the console shows this terminal's turns like any other
agent's. Leave that extension in place.
`;
}
