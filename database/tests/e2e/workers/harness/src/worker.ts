import { registerWorker, Logger } from 'iii-sdk';
import { resolve } from 'node:path';
import { Runner } from './runner.ts';
import type { DriverKey } from './dialect.ts';

const URL = process.env.III_URL ?? 'ws://127.0.0.1:49134';
const REPORT_PATH = resolve(process.env.HARNESS_REPORT_PATH ?? './reports/report.json');
const FILTER = process.env.HARNESS_FILTER as DriverKey | undefined;

const iii = registerWorker(URL);
const logger = new Logger();
const runner = new Runner({ iii, reportPath: REPORT_PATH, filterDriver: FILTER });

iii.registerFunction(
  'harness::on_outbox_row',
  async (payload: unknown) => runner.onOutboxBatch(payload),
  { description: 'Sink for iii-database::query-poll dispatches; routes by payload.db.' },
);

logger.info('harness: registered, kicking off suite', { url: URL, filter: FILTER ?? 'all', reportPath: REPORT_PATH });

(async () => {
  // ANSI colors only when stdout is a TTY — run-tests.sh redirects to a log file,
  // and bash's grep for the HARNESS_DONE sentinel must see plain text.
  const useColor = process.stdout.isTTY === true;
  const GREEN = useColor ? '\x1b[32m' : '';
  const RED = useColor ? '\x1b[31m' : '';
  const RESET = useColor ? '\x1b[0m' : '';
  let exitCode = 1;
  try {
    // Per-case results stream to stdout as they complete (see runner.ts).
    // Here we just wait for the run and emit the final sentinel.
    const { pass, total } = await runner.runAll();
    const status = pass === total ? 'PASS' : 'FAIL';
    const color = status === 'PASS' ? GREEN : RED;
    console.log(`HARNESS_DONE: ${color}${status}${RESET} ${pass}/${total}`);
    exitCode = status === 'PASS' ? 0 : 1;
  } catch (e: any) {
    console.error('[harness] fatal:', e?.stack ?? e);
    console.log(`HARNESS_DONE: ${RED}FAIL${RESET} 0/0`);
    exitCode = 1;
  }
  // runAll() called runner.unregisterAllTriggers() which writes UnregisterTrigger
  // messages to the websocket synchronously. The SDK's Trigger.unregister() is
  // fire-and-forget — sendMessage queues bytes but doesn't await the engine ACK.
  // Without this drain step, process.exit() terminates before the OS flushes
  // the TCP send buffer, the database worker never sees the unregister, and its
  // QueryPollTrigger tasks keep polling — causing the engine to log
  // "Function not found: harness::on_outbox_row" every 500ms until the worker
  // is restarted (or the next harness run evicts the zombie via trigger_id dedup).
  //
  // 200ms grace lets the OS flush ws bytes; iii.shutdown() then closes the ws
  // and drains OTel queues. iii.shutdown() itself does NOT await the ws close
  // handshake, hence the explicit delay.
  await new Promise((r) => setTimeout(r, 200));
  await iii.shutdown();
  process.exit(exitCode);
})();
