/**
 * Subscriber registry for the `harness::hook::pre-trigger` trigger
 * type. Hook owners bind via the engine's standard trigger registration
 * (`iii.registerTrigger({ type: "harness::hook::pre-trigger", … })`);
 * the engine routes each registration here. After a harness restart the
 * engine replays existing registrations to the type owner, so the set
 * rebuilds itself.
 */

import type { ISdk, TriggerConfig } from '../../runtime/iii.js';
import { logger } from '../../runtime/otel.js';
import { type BindingConfig, BindingConfigSchema, PRE_TRIGGER_TYPE, globMatch } from './types.js';

export type HookBinding = {
  id: string;
  function_id: string;
  config: BindingConfig;
};

const bindings = new Map<string, HookBinding>();

export function resetPreTriggerBindingsForTests(): void {
  bindings.clear();
}

export function addPreTriggerBindingForTests(binding: HookBinding): void {
  bindings.set(binding.id, binding);
}

function bindingMatches(binding: HookBinding, function_id: string): boolean {
  const globs = binding.config.functions;
  if (!globs || globs.length === 0) return true;
  return globs.some((pattern) => globMatch(pattern, function_id));
}

/** Bindings consulted for one trigger, in chain order. */
export function snapshotBindings(function_id: string): HookBinding[] {
  return [...bindings.values()]
    .filter((binding) => bindingMatches(binding, function_id))
    .sort(
      (a, b) =>
        (a.config.priority ?? 0) - (b.config.priority ?? 0) ||
        a.function_id.localeCompare(b.function_id),
    );
}

export function registerPreTriggerType(iii: ISdk): void {
  iii.registerTriggerType<unknown>(
    {
      id: PRE_TRIGGER_TYPE,
      description:
        'Synchronous pre-trigger hook point: bound functions are consulted before every ' +
        'agent function call and answer {decision: "continue" | "deny" | "hold"}. Config: ' +
        '{functions?: string[] (globs), priority?: number, timeout_ms?: number, ' +
        'on_error?: "fail_closed" | "fail_open"}.',
    },
    {
      async registerTrigger(config: TriggerConfig<unknown>) {
        // A throw here rejects the binding at registration time.
        const parsed = BindingConfigSchema.parse(config.config ?? {});
        bindings.set(config.id, { id: config.id, function_id: config.function_id, config: parsed });
        logger.info('pre_trigger hook bound', {
          id: config.id,
          function_id: config.function_id,
          functions: parsed.functions ?? ['*'],
        });
      },
      async unregisterTrigger(config: TriggerConfig<unknown>) {
        bindings.delete(config.id);
        logger.info('pre_trigger hook unbound', { id: config.id });
      },
    },
  );
}
