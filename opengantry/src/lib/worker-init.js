/** iii-sdk init for the OpenGantry control-plane worker only. */

export function opengantryWorkerOptions() {
  const otelEnabled = process.env.OTEL_ENABLED?.trim().toLowerCase();
  const otelOn =
    otelEnabled === 'true' || otelEnabled === '1' || otelEnabled === 'yes' || otelEnabled === 'on';

  return {
    workerName: 'opengantry',
    workerDescription: 'OpenGantry governance (verify, middleware, RBAC hooks)',
    otel: { enabled: otelOn },
  };
}
