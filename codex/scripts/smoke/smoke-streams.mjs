import { registerWorker } from 'iii-sdk';
const iii = registerWorker('ws://127.0.0.1:49134', { workerName: 'smoke-codex' });
await new Promise((r) => setTimeout(r, 1500));
const res = await iii.trigger({
  function_id: 'codex::run',
  payload: {
    prompt: 'Run the command `ls /tmp | head -3` and tell me how many lines it printed.',
    cwd: '/tmp',
    sandbox_mode: 'read-only',
  },
  timeoutMs: 240_000,
});
console.log('result ->', res.result, '| error:', res.is_error);
const raw = await iii.trigger({
  function_id: 'stream::list',
  payload: { stream_name: 'codex::events', group_id: res.session_id },
});
console.log('codex::events types ->', JSON.stringify(raw.map((f) => f.type)));
const agent = await iii.trigger({
  function_id: 'stream::list',
  payload: { stream_name: 'agent::events', group_id: res.session_id },
});
console.log('agent::events types ->', JSON.stringify(agent.map((f) => f.type)));
const exec = agent.find((f) => f.type === 'function_execution_start');
console.log('exec frame ->', JSON.stringify(exec ?? null).slice(0, 200));
process.exit(0);
