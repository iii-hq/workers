// MinIO bucket-notification → ElasticMQ shim. MinIO can't post directly to
// AWS SQS, but its webhook target sends an S3-event-shaped envelope whose
// `Records[]` array is byte-compatible with what the storage worker's SQS
// poller consumes (see storage/src/triggers/normalize.rs::extract_event,
// which dereferences `/Records/0/eventName`, `/Records/0/s3/bucket/name`,
// `/Records/0/s3/object/key`, etc.). We just relay the `Records` array as
// the SQS message body and let the worker do the rest.
//
// Why this exists at all: the storage worker's S3 trigger source long-polls
// SQS, and the e2e harness uses MinIO (no built-in SQS target). ElasticMQ
// is a drop-in SQS-compatible broker; this bridge is the missing arrow
// between MinIO's webhook target and ElasticMQ.
import { SQSClient, SendMessageCommand, CreateQueueCommand } from '@aws-sdk/client-sqs'

const SQS_ENDPOINT = process.env.SQS_ENDPOINT ?? 'http://elasticmq:9324'
const QUEUE_NAME = process.env.SQS_QUEUE_NAME ?? 'storage-events'
const SQS_QUEUE_URL = process.env.SQS_QUEUE_URL ?? `${SQS_ENDPOINT}/000000000000/${QUEUE_NAME}`
const PORT = Number(process.env.PORT ?? 8080)

const sqs = new SQSClient({
  endpoint: SQS_ENDPOINT,
  region: process.env.AWS_REGION ?? 'us-east-1',
  // ElasticMQ accepts any signature; placeholder creds satisfy the SDK
  // credential-chain requirement without leaking real AWS creds into tests.
  credentials: { accessKeyId: 'elasticmq', secretAccessKey: 'elasticmq' },
})

interface MinioEvent {
  EventName?: string
  Records?: unknown[]
}

// Create the queue on boot — idempotent in ElasticMQ; the worker's SQS
// poller and our SendMessage calls will both fail until this exists.
// Retry the call: ElasticMQ's healthcheck may flip green a hair before it
// actually accepts requests on cold boot.
async function ensureQueue(): Promise<void> {
  const deadline = Date.now() + 30_000
  let lastErr: unknown
  while (Date.now() < deadline) {
    try {
      await sqs.send(new CreateQueueCommand({ QueueName: QUEUE_NAME }))
      console.log(`[bridge] queue ensured: ${QUEUE_NAME}`)
      return
    } catch (e: unknown) {
      lastErr = e
      await new Promise((r) => setTimeout(r, 500))
    }
  }
  throw new Error(`could not create queue after 30s: ${String(lastErr)}`)
}

await ensureQueue()

const server = Bun.serve({
  port: PORT,
  async fetch(req: Request): Promise<Response> {
    const url = new URL(req.url)
    if (url.pathname === '/healthz') return new Response('ok', { status: 200 })
    if (req.method !== 'POST') return new Response('method not allowed', { status: 405 })
    let body: MinioEvent
    try {
      body = (await req.json()) as MinioEvent
    } catch (e: unknown) {
      return new Response(`bad json: ${String(e)}`, { status: 400 })
    }
    // MinIO sends a test payload without `Records` when an event target is
    // first registered — ack and drop so the registration succeeds.
    if (!Array.isArray(body.Records) || body.Records.length === 0) {
      return new Response('no records', { status: 200 })
    }
    // MinIO emits `eventName: "s3:ObjectCreated:Put"` whereas AWS S3 → SQS
    // omits the `s3:` prefix, and the storage worker's normalize_s3 uses
    // `event_name.starts_with("ObjectCreated:")`. Strip the prefix here so
    // the worker accepts MinIO-sourced events without the spec-correct
    // AWS-S3 normalizer needing a MinIO-shaped escape hatch.
    const normalized = body.Records.map((rec) => {
      const r = rec as Record<string, unknown>
      const en = typeof r.eventName === 'string' ? r.eventName : ''
      return en.startsWith('s3:') ? { ...r, eventName: en.slice('s3:'.length) } : r
    })
    try {
      await sqs.send(
        new SendMessageCommand({
          QueueUrl: SQS_QUEUE_URL,
          MessageBody: JSON.stringify({ Records: normalized }),
        }),
      )
      // Compact one-line trace per relayed event keeps the chain debuggable.
      const r0 = body.Records[0] as Record<string, any>
      const eventName = r0?.eventName ?? '?'
      const bucket = r0?.s3?.bucket?.name ?? '?'
      const key = r0?.s3?.object?.key ?? '?'
      console.log(`[bridge] ${eventName} ${bucket}/${key}`)
    } catch (e: unknown) {
      console.error(`[bridge] sqs send failed: ${String(e)}`)
      return new Response(`sqs send failed: ${String(e)}`, { status: 502 })
    }
    return new Response('ok', { status: 200 })
  },
})

console.log(`[bridge] listening on :${server.port}, forwarding to ${SQS_QUEUE_URL}`)
