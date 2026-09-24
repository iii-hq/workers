import { parseArgs } from 'node:util';
import { startWorker } from './worker.js';

const { values } = parseArgs({ options: { url: { type: 'string' } }, strict: true });
const worker = startWorker(values.url);
process.once('SIGINT', () => {
  void worker.shutdown();
});
process.once('SIGTERM', () => {
  void worker.shutdown();
});
