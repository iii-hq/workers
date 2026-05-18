import { loadConfig } from '../runtime/config.js';
import type { ISdk } from '../runtime/iii.js';
import { register as registerAgentCall } from './agent-call.js';
import * as bootstrap from './bootstrap.js';
import { loadOrchestratorConfig } from './config.js';
import {
  CONDITION_FN_ID as ABORT_CONDITION_FN,
  HANDLER_FN_ID as ABORT_HANDLER_FN,
  handleAbortSignalWrite,
  isAbortSignalWrite,
} from './on-abort-signal.js';
import {
  CONDITION_FN_ID as RECORD_CONDITION_FN,
  HANDLER_FN_ID as RECORD_HANDLER_FN,
  handleStepableRecordWrite,
  isStepableRecordWrite,
} from './on-record-written.js';
import {
  CONDITION_FN_ID as TERMINAL_CONDITION_FN,
  HANDLER_FN_ID as TERMINAL_HANDLER_FN,
  handleTerminalStateWrite,
  isTerminalStateWrite,
} from './on-terminal.js';
import { register as registerRunStart } from './run-start.js';
import { register as registerSubscriber } from './subscriber.js';

export async function register(iii: ISdk, ctx: { configPath: string }): Promise<void> {
  const cfg = await loadConfig(ctx.configPath);
  const orchestratorCfg = loadOrchestratorConfig(cfg);
  registerRunStart(iii, orchestratorCfg);
  registerAgentCall(iii);
  registerSubscriber(iii, orchestratorCfg);

  // Reactive abort wake. Mirrors the pattern in
  // harness/fanout/sessions-poll.ts (post c59210788c).
  iii.registerFunction(ABORT_CONDITION_FN, async (event: unknown) => isAbortSignalWrite(event), {
    description:
      'Condition: state event sets session/<id>/abort_signal = true (state:created or state:updated).',
  });

  iii.registerFunction(
    ABORT_HANDLER_FN,
    async (event: unknown) => handleAbortSignalWrite(iii, event),
    {
      description:
        'State trigger adapter on scope=agent for abort_signal writes; publishes turn::step_requested so the orchestrator picks up the abort promptly.',
    },
  );

  iii.registerTrigger({
    type: 'state',
    function_id: ABORT_HANDLER_FN,
    config: {
      scope: 'agent',
      condition_function_id: ABORT_CONDITION_FN,
    },
  });

  // Reactive `run::start_and_wait` wake. The condition matches terminal
  // turn_state writes; the handler resolves the in-process waiter
  // installed by executeSync. Replaces the old sync_poll_interval_ms
  // sleep loop.
  iii.registerFunction(
    TERMINAL_CONDITION_FN,
    async (event: unknown) => isTerminalStateWrite(event),
    {
      description: 'Condition: state event sets session/<id>/turn_state to state="stopped".',
    },
  );

  iii.registerFunction(
    TERMINAL_HANDLER_FN,
    async (event: unknown) => {
      handleTerminalStateWrite(event);
    },
    {
      description:
        'State trigger adapter on scope=agent for terminal turn_state writes; resolves the run::start_and_wait waiter for that session.',
    },
  );

  iii.registerTrigger({
    type: 'state',
    function_id: TERMINAL_HANDLER_FN,
    config: {
      scope: 'agent',
      condition_function_id: TERMINAL_CONDITION_FN,
    },
  });

  // Reactive turn-step wake. The condition matches non-terminal,
  // non-awaiting writes to session/<id>/turn_state; the handler
  // invokes turn::step for that session. Replaces the imperative
  // publishStep self-publish that used to live in subscriber.ts.
  iii.registerFunction(
    RECORD_CONDITION_FN,
    async (event: unknown) => isStepableRecordWrite(event),
    {
      description:
        'Condition: state event sets session/<id>/turn_state to a stepable state (excludes stopped + function_awaiting_approval).',
    },
  );

  iii.registerFunction(
    RECORD_HANDLER_FN,
    async (event: unknown) => handleStepableRecordWrite(iii, event),
    {
      description:
        'State trigger adapter on scope=agent for stepable turn_state writes; invokes turn::step. Replaces the imperative publishStep self-publish.',
    },
  );

  iii.registerTrigger({
    type: 'state',
    function_id: RECORD_HANDLER_FN,
    config: {
      scope: 'agent',
      condition_function_id: RECORD_CONDITION_FN,
    },
  });

  // Bootstrap best-effort skill download in the background.
  void bootstrap.run(iii, orchestratorCfg);
}
