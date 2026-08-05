// code-runner eval wrapper — planted at runtime creation. Do not edit in
// place. Runs an eval file as `node <file>` would, with the `iii` global (a
// lazy handle on the real iii-sdk client — see iii.mjs) installed first. If
// the eval used iii, the client is shut down after the run so the process
// can exit.
import { pathToFileURL } from 'node:url';
import { makeIii } from './iii.mjs';

const target = process.argv[2];
if (!target) {
  process.stderr.write('code-runner eval wrapper: missing target file argument\n');
  process.exit(1);
}
const { iii, client } = await makeIii();
globalThis.iii = iii;
// [node, eval.mjs, file] -> [node, file]: the evaluated file sees the argv
// a direct run would have given it.
process.argv.splice(1, 2, target);
try {
  await import(pathToFileURL(target).href);
} finally {
  const c = client();
  if (c) {
    await Promise.race([
      c.shutdown().catch(() => {}),
      new Promise((r) => setTimeout(r, 3000).unref()),
    ]);
  }
}
