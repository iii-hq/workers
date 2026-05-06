import { registerWorker, Logger } from 'iii-sdk';
import { resolve } from 'node:path';
import { Runner } from './runner.ts';
import { setupSandboxMocks } from './cases-fs-sandbox.ts';

const URL = process.env.III_URL ?? 'ws://127.0.0.1:49134';
const REPORT_PATH = resolve(process.env.HARNESS_REPORT_PATH ?? './reports/report.json');

const iii = registerWorker(URL);
const logger = new Logger();

process.on('unhandledRejection', (reason) => {
  const msg = (reason as { message?: string })?.message ?? String(reason);
  logger.warn('unhandledRejection (suppressed by harness)', { reason: msg });
});

setupSandboxMocks(iii);

const runner = new Runner({ iii, reportPath: REPORT_PATH });

logger.info('harness: registered, kicking off suite', { url: URL, reportPath: REPORT_PATH });

(async () => {
  const useColor = process.stdout.isTTY === true;
  const GREEN = useColor ? '\x1b[32m' : '';
  const RED = useColor ? '\x1b[31m' : '';
  const RESET = useColor ? '\x1b[0m' : '';
  let exitCode = 1;
  try {
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
  await new Promise((r) => setTimeout(r, 200));
  await iii.shutdown();
  process.exit(exitCode);
})();
