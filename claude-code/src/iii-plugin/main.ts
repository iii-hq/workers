#!/usr/bin/env node
/** The iii plugin's command line; see `./cli.ts` for the commands. */

import { runCli } from './cli.js';

/** Writes still in flight: a pipe write is asynchronous on POSIX. */
const pending: Promise<void>[] = [];
const writer = (stream: NodeJS.WriteStream) => (text: string) => {
  pending.push(new Promise((resolve) => stream.write(text, () => resolve())));
};

const code = await runCli(process.argv.slice(2), {
  env: process.env,
  fetchFn: fetch,
  stdout: writer(process.stdout),
  stderr: writer(process.stderr),
});
// Exit only once the hook's output is written: exiting earlier can cut it
// off, and waiting for a natural exit could hang on an idle keep-alive socket.
await Promise.all(pending);
process.exit(code);
