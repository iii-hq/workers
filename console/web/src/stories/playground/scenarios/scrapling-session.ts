import {
  scraplingSessionClose,
  scraplingSessionFetch,
  scraplingSessionList,
  scraplingSessionOpen,
} from '@/stories/fixtures/scrapling-fixtures'
import {
  makeBackend,
  streamAssistant,
  streamFcall,
  streamThought,
} from './helpers'

export const scraplingSession = makeBackend(
  'scrapling-session',
  async function* (_prompt, _mode, _model, opts) {
    const signal = opts?.signal
    yield* streamThought(
      'opening a stealth session, reusing it across fetches, then closing it…',
      { signal },
    )
    yield* streamFcall({
      functionId: 'scrapling::session-open',
      input: scraplingSessionOpen.input,
      output: scraplingSessionOpen.output,
      pendingApproval: true,
      approvalWaitMs: 1400,
      waitMs: 800,
      signal,
    })
    yield* streamFcall({
      functionId: 'scrapling::session-fetch',
      input: scraplingSessionFetch.input,
      output: scraplingSessionFetch.output,
      pendingApproval: true,
      approvalWaitMs: 1200,
      waitMs: 600,
      signal,
    })
    yield* streamFcall({
      functionId: 'scrapling::session-list',
      input: scraplingSessionList.input,
      output: scraplingSessionList.output,
      waitMs: 250,
      signal,
    })
    yield* streamFcall({
      functionId: 'scrapling::session-close',
      input: scraplingSessionClose.input,
      output: scraplingSessionClose.output,
      waitMs: 250,
      signal,
    })
    yield* streamAssistant(
      'reused one stealth session across the fetch (cookies persisted), then closed it.',
      { signal },
    )
  },
)
