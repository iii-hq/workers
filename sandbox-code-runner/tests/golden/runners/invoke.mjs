// sandbox-code-runner invoke runner — planted at runtime creation. Do not edit in place.
// Protocol: argv = [source_path]; stdin = JSON envelope
// {"sentinel": "<uuid>", "payload": <payload>}, consumed before the
// handler's source ever loads. Result = JSON printed after a line holding
// only the sentinel. Exit 0 = result, exit 1 = {"error": "..."}. A
// malformed/missing envelope has no sentinel to frame a reply with: it is
// reported on stderr and the process exits non-zero with no stdout at all.
// Handlers get the same `iii` global run code gets, built by the
// sibling iii.mjs (planted next to this file at runtime creation).
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { makeIii } from './iii.mjs';

async function main() {
  const [source] = process.argv.slice(2);
  const raw = readFileSync(0, 'utf8');
  let envelope = null;
  try {
    envelope = JSON.parse(raw);
  } catch {
    envelope = null;
  }
  if (envelope === null || typeof envelope !== 'object' || typeof envelope.sentinel !== 'string') {
    process.stderr.write(
      'sandbox-code-runner invoke runner: malformed envelope on stdin (expected {"sentinel": "...", "payload": ...})\n'
    );
    process.exitCode = 1;
    return;
  }
  const { sentinel, payload } = envelope;

  const { iii, client } = await makeIii();
  globalThis.iii = iii;

  let body;
  let code;
  try {
    const mod = await import(pathToFileURL(source).href);
    if (typeof mod.handler !== 'function') {
      throw new TypeError("source must export a function named 'handler(payload)'");
    }
    const out = await mod.handler(payload);
    body = JSON.stringify(out === undefined ? null : out);
    if (body === undefined) {
      throw new TypeError('handler result is not JSON-serializable');
    }
    code = 0;
  } catch (e) {
    body = JSON.stringify({ error: String((e && e.message) || e) });
    code = 1;
  }

  // Shut the iii client down BEFORE the frame, so any output it produces
  // lands in the logs region and never after the result line — and so the
  // process can exit without the exec timing out on an open socket. Capped
  // and unref'd for the same reason as the run wrapper's.
  const c = client();
  if (c) {
    await Promise.race([
      c.shutdown().catch(() => {}),
      new Promise((r) => setTimeout(r, 2000).unref()),
    ]);
  }

  process.stdout.write('\n' + sentinel + '\n' + body + '\n');
  process.exitCode = code;
}

await main();
