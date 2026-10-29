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

// The engine's `on_observer_registration_function_id` input. Passthrough: the
// engine owns the envelope and may add fields.
export const ObserverRegistrationRequestSchema = z
  .object({
    observer_id: z.string(),
    function_id: z.string(),
    namespace: z.string().optional(),
    args: z.record(z.string(), z.unknown()).optional(),
    filters: z.record(z.string(), z.unknown()).nullable().optional(),
    target_function_id: z.string(),
    target_namespace: z.string().optional(),
    metadata: z.record(z.string(), z.unknown()).nullable().optional(),
    context: z.record(z.string(), z.unknown()).nullable().optional(),
  })
  .passthrough();

// An accepted observer is answered with the empty object, which rewrites
// nothing; a denial throws GantryDenied.
export const ObserverRegistrationResponseSchema = z.object({}).strict();

// What gantry::on-verdict observers observe: one event per completed verify.
export const VerdictEventSchema = z
  .object({
    status: z.string(),
    error_code: z.string().nullable(),
    repo_root: z.string().nullable(),
    msn_id: z.string().nullable(),
    mission_rel_path: z.string().nullable(),
  })
  .strict();

export const FUNCTION_FORMATS = {
  'gantry::middleware': {
    request: MiddlewareRequestSchema,
    response: MiddlewareResponseSchema,
  },
  'gantry::verify': {
    description:
      "Verify a mission by running the repo's own build and test gates. Returns a signed verdict token bound to that mission revision, which promote-class calls must carry.",
    request: VerifyRequestSchema,
    response: VerifyResponseSchema,
  },
  'gantry::on-function-registration': {
    request: FunctionIdEnvelopeSchema,
    response: FunctionIdEnvelopeSchema,
  },
  'gantry::on-observer-registration': {
    request: ObserverRegistrationRequestSchema,
    response: ObserverRegistrationResponseSchema,
  },
  'gantry::on-verdict': {
    description:
      'Called once per completed gantry::verify with its verdict: { status, error_code, repo_root, msn_id, mission_rel_path }. Private: observe it; only opengantry calls it.',
    request: VerdictEventSchema,
    response: VerdictEventSchema,
    private: true,
  },
};

export function formatFor(schema) {
  return jsonSchema(schema);
}
