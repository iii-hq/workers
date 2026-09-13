const text = { type: 'string' };
const boolean = { type: 'boolean' };
const list = { type: 'array', items: text };
function object(properties: Record<string, unknown>, required = Object.keys(properties)) {
  return { type: 'object' as const, properties, required, additionalProperties: false };
}
const did = {
  type: 'string',
  pattern: '^did:key:z[1-9A-HJ-NP-Za-km-z]{47}$',
  description:
    'Independently expected Ed25519 did:key; never derive this choice from the supplied credential.',
};
export const preflightRequest = object({
  url: {
    type: 'string',
    minLength: 1,
    maxLength: 2048,
    description:
      'Exact caller-selected public HTTP(S) DNS endpoint. Guild actively probes it; exclude secrets.',
  },
});
export const passportRequest = object({
  credential: {
    type: 'object',
    additionalProperties: true,
    description:
      'Complete supplied PUBLIC AgentGuildPassport JSON object, at most 32 KiB, sent unchanged to Guild. Never confidential claims.',
  },
  expectedIssuerDid: did,
  expectedSubjectDid: did,
});
const provenance = {
  serviceOrigin: { const: 'https://agent-guild-5d5r.onrender.com' },
  requestedAt: text,
  completedAt: text,
  httpStatus: { const: 200 },
  responseBytes: { type: 'integer', minimum: 0, maximum: 65536 },
};
const failure = object({
  operation: { enum: ['endpoint_observation', 'passport_verification'] },
  status: { enum: ['rejected', 'unavailable'] },
  code: text,
  verified: { const: false },
  serviceOrigin: provenance.serviceOrigin,
});
export const preflightResponse = {
  oneOf: [
    object({
      operation: { const: 'endpoint_observation' },
      status: { const: 'observed' },
      ...provenance,
      target: text,
      verdict: { enum: ['do_not_delegate', 'delegate_with_caution', 'no_failed_checks'] },
      checks: {
        type: 'array',
        minItems: 6,
        maxItems: 6,
        items: object({
          check: {
            enum: [
              'endpoint_reachable',
              'protocol_handshake',
              'agent_card_resolves',
              'agent_card_signed',
              'payment_claim_holds',
              'independent_evidence',
            ],
          },
          status: { enum: ['proven', 'failed', 'unknown'] },
        }),
      },
      failed: list,
      unknowns: list,
      limitations: list,
    }),
    failure,
  ],
};
export const passportResponse = {
  oneOf: [
    object({
      operation: { const: 'passport_verification' },
      status: { const: 'completed' },
      ...provenance,
      issuerDid: did,
      subjectDid: did,
      signatureValidReported: boolean,
      guildIssuedReported: boolean,
      verified: boolean,
      validFrom: text,
      validUntil: text,
      timeAndBindingChecksPassed: { const: true },
      limitations: list,
    }),
    failure,
  ],
};
