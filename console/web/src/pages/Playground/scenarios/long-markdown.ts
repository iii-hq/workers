import { makeBackend, streamAssistant } from './helpers'

const BODY = `# system overview

a deeper walkthrough of the architecture, written long-form so the markdown
renderer has plenty of surface to chew on. paragraphs interleave with lists,
tables, fenced code, and quotes.

## components

each component owns one job and exposes one boundary. the boundaries are the
contract; everything else is implementation detail.

1. **gateway** — terminates tls, authenticates, rate-limits.
   1. tls is offloaded to the load balancer; gateway sees plaintext.
   2. auth is jwt verification with a rolling jwks cache.
      - cache hits short-circuit the call to the issuer.
      - cache misses fall through to the issuer with a 200ms timeout.
   3. rate-limits are token-bucket, keyed on (subject, route).
2. **router** — map verb + path to a handler ref.
3. **handler** — pure function over (request, deps) → response.
4. **store** — the only stateful component; the rest are stateless workers.

## a worked example

> "show me what happens when a user creates a new conversation."

the gateway accepts the request, validates the bearer token, checks the
rate-limit, and forwards the validated envelope to the router. the router
matches \`POST /conversations\` to the create-conversation handler, which
inserts a row in \`conversations\`, appends a system event, and returns the
new id.

\`\`\`ts
// handlers/conversations.ts
export async function create(
  req: ValidatedRequest<CreateConversationInput>,
  { db, clock }: Deps,
): Promise<Response<CreateConversationOutput>> {
  const id = ulid()
  await db.tx(async (tx) => {
    await tx.run(insertConversation, { id, userId: req.subject, createdAt: clock.now() })
    await tx.run(insertEvent, { conversationId: id, kind: 'created' })
  })
  return ok({ id })
}
\`\`\`

\`\`\`sql
-- migrations/0001_conversations.sql
create table conversations (
  id           text primary key,
  user_id      text not null,
  title        text,
  created_at   timestamp not null,
  updated_at   timestamp not null
);

create index conversations_user_id_idx on conversations (user_id, updated_at desc);
\`\`\`

\`\`\`bash
$ curl -X POST https://api.example.com/conversations \\
    -H 'authorization: bearer $TOKEN' \\
    -H 'content-type: application/json' \\
    -d '{}'
{"id":"01HV2..."}
\`\`\`

## error semantics

| layer    | shape                  | retry?                     |
|----------|------------------------|----------------------------|
| gateway  | http 4xx               | no — caller fix            |
| gateway  | http 5xx               | yes — exponential backoff  |
| router   | not_found              | no                         |
| handler  | domain.error           | depends on \`kind\`          |
| store    | conflict / deadlock    | yes — bounded retry        |

domain errors carry a stable \`kind\` so callers can branch on the contract:

- \`kind: 'rate_limited'\` — back off and retry with jitter.
- \`kind: 'unauthorized'\` — refresh the token, retry once.
- \`kind: 'invalid_argument'\` — caller bug; do not retry.
- \`kind: 'conflict'\` — deterministic conflict; refetch and replay.

## checklist before shipping

- [ ] every public endpoint has a test exercising the unhappy path.
- [ ] every domain error has a stable \`kind\` and is documented.
- [ ] migrations are reversible.
- [x] release notes are written.
- [ ] dashboards show p50/p95/p99 latency for the new route.

> *the goal isn't to anticipate every failure — it's to make every failure
> debuggable when it happens.*`

export const longMarkdown = makeBackend(
  'long-markdown',
  async function* (_prompt, _mode, _model, opts) {
    yield* streamAssistant(BODY, { signal: opts?.signal, meanDelayMs: 6 })
  },
)
