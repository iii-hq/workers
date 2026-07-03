import {
  scraplingCssAll,
  scraplingFetchExtract,
  scraplingStealthyCloudflare,
} from '@/stories/fixtures/scrapling-fixtures'
import {
  makeBackend,
  streamAssistant,
  streamFcall,
  streamThought,
} from './helpers'

export const scraplingScrape = makeBackend(
  'scrapling-scrape',
  async function* (_prompt, _mode, _model, opts) {
    const signal = opts?.signal
    yield* streamThought(
      'fetching the catalog with scrapling, then extracting the fields…',
      { signal },
    )
    yield* streamFcall({
      functionId: 'scrapling::stealthy-fetch',
      input: scraplingStealthyCloudflare.input,
      output: scraplingStealthyCloudflare.output,
      pendingApproval: true,
      approvalWaitMs: 1600,
      waitMs: 900,
      signal,
    })
    yield* streamFcall({
      functionId: 'scrapling::fetch',
      input: scraplingFetchExtract.input,
      output: scraplingFetchExtract.output,
      waitMs: 700,
      signal,
    })
    yield* streamFcall({
      functionId: 'scrapling::css',
      input: scraplingCssAll.input,
      output: scraplingCssAll.output,
      waitMs: 350,
      signal,
    })
    yield* streamAssistant(
      'scraped the catalog: 3 protected items, plus title + links from the books page.',
      { signal },
    )
  },
)
