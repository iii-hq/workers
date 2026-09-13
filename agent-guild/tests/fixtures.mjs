import { readFileSync } from 'node:fs';

const alphabet = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
export function encode(bytes) {
  let n = BigInt(`0x${Buffer.from(bytes).toString('hex')}`);
  let s = '';
  while (n) {
    s = alphabet[Number(n % 58n)] + s;
    n /= 58n;
  }
  for (const b of bytes) {
    if (b !== 0) break;
    s = `1${s}`;
  }
  return s;
}
export const issuer = `did:key:z${encode([0xed, 1, ...new Array(32).fill(1)])}`;
export const subject = `did:key:z${encode([0xed, 1, ...new Array(32).fill(2)])}`;
export const target = 'https://agent-guild-5d5r.onrender.com/mcp';
export const observed = () =>
  JSON.parse(readFileSync(new URL('./fixtures/preflight.raw', import.meta.url), 'utf8'));
export function passport(now = Date.now()) {
  return {
    '@context': ['https://www.w3.org/ns/credentials/v2'],
    type: ['VerifiableCredential', 'AgentGuildPassport'],
    issuer,
    credentialSubject: {
      id: subject,
      publicClaim: { note: 'synthetic fixture only', unknown: null },
    },
    validFrom: new Date(now - 1000).toISOString(),
    validUntil: new Date(now + 60000).toISOString(),
    proof: {
      type: 'DataIntegrityProof',
      cryptosuite: 'eddsa-jcs-2022',
      proofPurpose: 'assertionMethod',
      verificationMethod: `${issuer}#${issuer.slice(8)}`,
      proofValue: `z${encode(new Array(64).fill(3))}`,
    },
  };
}
export const passportInput = () => ({
  credential: passport(),
  expectedIssuerDid: issuer,
  expectedSubjectDid: subject,
});
export const verifier = (valid = true, guild = true) => ({
  valid,
  guild_issued: guild,
  issuer,
  subject_did: subject,
});
export const jsonResponse = (value) =>
  new Response(JSON.stringify(value), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  });
export function corrected(value) {
  value.failed = value.checks.filter((c) => c.status === 'failed').map((c) => c.check);
  value.unknowns = value.checks.filter((c) => c.status === 'unknown').map((c) => c.check);
  value.scored = value.checks.filter((c) => c.status !== 'unknown').map((c) => c.check);
  value.verdict = value.failed.some((c) => ['endpoint_reachable', 'protocol_handshake'].includes(c))
    ? 'do_not_delegate'
    : value.failed.length
      ? 'delegate_with_caution'
      : 'no_failed_checks';
  return value;
}
