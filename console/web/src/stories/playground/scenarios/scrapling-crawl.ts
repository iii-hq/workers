import { scraplingCrawl } from '@/stories/fixtures/scrapling-fixtures'
import {
  makeBackend,
  streamAssistant,
  streamFcall,
  streamThought,
} from './helpers'

export const scraplingCrawlScenario = makeBackend(
  'scrapling-crawl',
  async function* (_prompt, _mode, _model, opts) {
    const signal = opts?.signal
    yield* streamThought('crawling the blog, following same-domain links…', {
      signal,
    })
    yield* streamFcall({
      functionId: 'scrapling::crawl',
      input: scraplingCrawl.input,
      output: scraplingCrawl.output,
      pendingApproval: true,
      approvalWaitMs: 1400,
      waitMs: 1200,
      signal,
    })
    yield* streamAssistant(
      'crawled 12 pages, extracted 11 titles (1 dead link). Full items are on the stream.',
      { signal },
    )
  },
)
