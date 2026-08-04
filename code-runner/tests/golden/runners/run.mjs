// code-runner runner — planted at runtime creation. Do not edit in place.
// Protocol: argv = [source_path]; stdin = JSON envelope
// {"sentinel": "<uuid>", "payload": <payload>}, consumed before the
// handler's source ever loads. Result = JSON printed after a line holding
// only the sentinel. Exit 0 = result, exit 1 = {"error": "..."}. A
// malformed/missing envelope has no sentinel to frame a reply with: it is
// reported on stderr and the process exits non-zero with no stdout at all.
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

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
      'code-runner runner: malformed envelope on stdin (expected {"sentinel": "...", "payload": ...})\n'
    );
    process.exitCode = 1;
    return;
  }
  const { sentinel, payload } = envelope;

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
  process.stdout.write('\n' + sentinel + '\n' + body + '\n');
  process.exitCode = code;
}

await main();
