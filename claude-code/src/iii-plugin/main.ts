#!/usr/bin/env node
/** The iii plugin's command line; see `./cli.ts` for the commands. */

import { runCli } from './cli.js';

const code = await runCli(process.argv.slice(2), {
  env: process.env,
  fetchFn: fetch,
  stdout: (text) => process.stdout.write(text),
  stderr: (text) => process.stderr.write(text),
});
process.exit(code);
