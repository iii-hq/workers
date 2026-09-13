import { observation, verification } from './projection.js';
import { type AgentGuildOptions, requestJson } from './transport.js';
import {
  credentialObject,
  did,
  endpoint,
  GuildError,
  inputObject,
  ORIGIN,
  passportBinding,
  requireValue,
} from './validation.js';

export type { AgentGuildOptions } from './transport.js';

export function createOperations(options: AgentGuildOptions = {}) {
  const timeoutMs = options.timeoutMs ?? 15000;
  const maxAge = options.maxPassportAgeSeconds ?? 86400;
  requireValue(
    Number.isInteger(timeoutMs) && timeoutMs >= 100 && timeoutMs <= 45000,
    'invalid_config',
  );
  requireValue(Number.isInteger(maxAge) && maxAge >= 1 && maxAge <= 604800, 'invalid_config');
  requireValue(
    options.allowedHosts === undefined ||
      (Array.isArray(options.allowedHosts) &&
        options.allowedHosts.length <= 64 &&
        options.allowedHosts.every(
          (host) => typeof host === 'string' && endpoint(`https://${host}/`) === host,
        )),
    'invalid_config',
  );
  const hosts = options.allowedHosts === undefined ? undefined : new Set(options.allowedHosts);
  const allowedIssuer =
    options.expectedIssuerDid === undefined ? undefined : did(options.expectedIssuerDid);
  const fetcher = options.fetch ?? globalThis.fetch?.bind(globalThis);
  requireValue(typeof fetcher === 'function', 'fetch_unavailable');
  return {
    async preflight(input: unknown, signal?: AbortSignal) {
      try {
        const { url } = inputObject(input, ['url']);
        requireValue(typeof url === 'string', 'invalid_input');
        const host = endpoint(url);
        requireValue(hosts === undefined || hosts.has(host), 'host_not_approved');
        const response = await requestJson(
          fetcher,
          `/preflight?url=${encodeURIComponent(url)}`,
          timeoutMs,
          undefined,
          signal,
        );
        return {
          operation: 'endpoint_observation' as const,
          status: 'observed' as const,
          serviceOrigin: ORIGIN,
          requestedAt: response.requestedAt,
          completedAt: response.completedAt,
          httpStatus: response.httpStatus,
          responseBytes: response.responseBytes,
          ...observation(response.value, url),
        };
      } catch (error) {
        return failure('endpoint_observation', error);
      }
    },
    async verifyPassport(input: unknown, signal?: AbortSignal) {
      try {
        const parsed = inputObject(input, [
          'credential',
          'expectedIssuerDid',
          'expectedSubjectDid',
        ]);
        const credential = credentialObject(parsed.credential);
        const issuer = did(parsed.expectedIssuerDid);
        const subject = did(parsed.expectedSubjectDid);
        requireValue(
          allowedIssuer === undefined || allowedIssuer === issuer,
          'issuer_not_approved',
        );
        passportBinding(credential, issuer, subject, maxAge, Date.now());
        const response = await requestJson(
          fetcher,
          '/credentials/verify',
          timeoutMs,
          JSON.stringify(credential),
          signal,
        );
        passportBinding(credential, issuer, subject, maxAge, Date.now());
        return {
          operation: 'passport_verification' as const,
          status: 'completed' as const,
          serviceOrigin: ORIGIN,
          requestedAt: response.requestedAt,
          completedAt: response.completedAt,
          httpStatus: response.httpStatus,
          responseBytes: response.responseBytes,
          ...verification(response.value, credential, issuer, subject),
        };
      } catch (error) {
        return failure('passport_verification', error);
      }
    },
  };
}
function failure(operation: 'endpoint_observation' | 'passport_verification', error: unknown) {
  const code = error instanceof GuildError ? error.code : 'operation_unavailable';
  const transport = [
    'http_unavailable',
    'request_unavailable',
    'request_aborted',
    'request_timeout',
  ].includes(code);
  return {
    operation,
    status: transport ? ('unavailable' as const) : ('rejected' as const),
    code,
    verified: false,
    serviceOrigin: ORIGIN,
  };
}
