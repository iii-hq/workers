import type { RemoteFunctionHandler, RegisterFunctionOptions } from 'iii-sdk';
import { z } from 'zod';
import { zodToJsonSchema } from 'zod-to-json-schema';
import type { ISdk } from '../../runtime/iii.js';
import type { PermissionsHandle } from './handle.js';
import type { Decision, MatchedConstraint } from './types.js';

const CheckPermissionsPayloadSchema = z.object({
  function_id: z.string().min(1),
  args: z.record(z.any()).optional(),
});

export type CheckPermissionsPayload = z.infer<typeof CheckPermissionsPayloadSchema>;

export type PolicyCheckReply =
  | { decision: 'allow'; rule_id: string }
  | {
      decision: 'deny';
      rule_id: string;
      rule_action: 'deny';
      matched_constraint?: MatchedConstraint;
    }
  | { decision: 'needs_approval' };

export function decisionToValue(decision: Decision): PolicyCheckReply {
  switch (decision.kind) {
    case 'allow':
      return { decision: 'allow', rule_id: decision.rule_id };
    case 'deny':
      return {
        decision: 'deny',
        rule_id: decision.rule_id,
        rule_action: 'deny',
        ...(decision.matched_constraint ? { matched_constraint: decision.matched_constraint } : {}),
      };
    default:
      return { decision: 'needs_approval' };
  }
}

const checkPermissionsFunctionOptions = {
  description:
    'Check a function call against iii-permissions.yaml; returns allow, deny, or needs_approval.',
  request_format: zodToJsonSchema(CheckPermissionsPayloadSchema, {
    name: 'CheckPermissionsPayload',
  }),
} as RegisterFunctionOptions;

export function registerPolicy(iii: ISdk, handle: PermissionsHandle): void {
  const handler: RemoteFunctionHandler<CheckPermissionsPayload, PolicyCheckReply> = async (
    input,
  ) => {
    const { function_id, args } = CheckPermissionsPayloadSchema.parse(input);
    return decisionToValue(handle.current().check(function_id, args ?? {}));
  };

  iii.registerFunction('policy::check_permissions', handler, checkPermissionsFunctionOptions);
}
