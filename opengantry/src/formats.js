/**
 * Zod schemas compiled to JSON Schema for registerFunction. The iii
 * contract requires request_format and response_format on every
 * registration. Payload/context .passthrough() extra adopter fields
 * (branch, tokens, custom context). The request envelope stays .strict().
 */
import { z } from 'zod';

function jsonSchema(schema) {
  const out = z.toJSONSchema(schema);
  delete out.$schema;
  return out;
}

const MiddlewarePayloadSchema = z
  .object({
    verdict_token: z.string().optional(),
    branch: z.string().optional(),
  })
  .passthrough();

const MiddlewareContextSchema = z
  .object({
    msn_id: z.string().optional(),
    holder_id: z.string().optional(),
    repo_root: z.string().optional(),
    worktree_path: z.string().optional(),
    mission_rel_path: z.string().optional(),
    mission_rel: z.string().optional(),
    verdict_token: z.string().optional(),
  })
  .passthrough();

export const MiddlewareRequestSchema = z
  .object({
    function_id: z.string(),
    payload: MiddlewarePayloadSchema.optional(),
    context: MiddlewareContextSchema.optional(),
  })
  .strict();

// The gate is a pass-through: on allow, the response is whatever the forwarded
// call returned. Typed as an open object rather than z.unknown() because
// z.toJSONSchema(z.unknown()) is `{}`, which the registry renders as "unknown"
// and `collect_worker_interface.py --assert-typed-schemas` rejects at publish.
// Same shape openwiki uses for open payloads. Widen to anyOf if a governed
// function ever returns a bare scalar or array.
export const MiddlewareResponseSchema = z.record(z.string(), z.unknown());

export const VerifyRequestSchema = z
  .object({
    repo_root: z.string(),
    msn_id: z.string().optional(),
    mission_rel_path: z.string().optional(),
    options: z.record(z.string(), z.unknown()).optional(),
  })
  .strict();

const VerifyFindingSchema = z
  .object({
    failed_gate: z.string().optional(),
    offending_file: z.string().optional(),
    line: z.number().optional(),
    severity: z.string().optional(),
    resolution_hint: z.string().optional(),
  })
  .strict();

export const VerifyResponseSchema = z
  .object({
    status: z.string(),
    phase: z.string().optional(),
    message: z.string().optional(),
    error_code: z.string().optional(),
    exit_code: z.number().optional(),
    msn_id: z.string().optional(),
    mission_file_path: z.string().optional(),
    findings: z.array(VerifyFindingSchema).optional(),
    fix_hints: z.array(z.string()).optional(),
    next_actions: z.array(z.string()).optional(),
  })
  .strict();

export const FunctionIdEnvelopeSchema = z
  .object({
    function_id: z.string(),
  })
  .strict();

export const OnTriggerTypeRegistrationRequestSchema = z
  .object({
    type_id: z.string(),
  })
  .strict();

// onTriggerTypeRegistration is an unconditional deny — it always throws
// GantryDenied and has no success shape. Declared as the empty strict object so
// the schema stays typed (see the note on MiddlewareResponseSchema).
export const OnTriggerTypeRegistrationResponseSchema = z.object({}).strict();

export const FUNCTION_FORMATS = {
  'gantry::middleware': {
    request: MiddlewareRequestSchema,
    response: MiddlewareResponseSchema,
  },
  'gantry::verify': {
    request: VerifyRequestSchema,
    response: VerifyResponseSchema,
  },
  'gantry::on-function-registration': {
    request: FunctionIdEnvelopeSchema,
    response: FunctionIdEnvelopeSchema,
  },
  'gantry::on-trigger-registration': {
    request: FunctionIdEnvelopeSchema,
    response: FunctionIdEnvelopeSchema,
  },
  'gantry::on-trigger-type-registration': {
    request: OnTriggerTypeRegistrationRequestSchema,
    response: OnTriggerTypeRegistrationResponseSchema,
  },
};

export function formatFor(schema) {
  return jsonSchema(schema);
}
