import {
  scraplingDescribe,
  scraplingFindByText,
  scraplingToMarkdown,
} from '@/stories/fixtures/scrapling-fixtures'
import {
  makeBackend,
  streamAssistant,
  streamFcall,
  streamThought,
} from './helpers'

export const scraplingParse = makeBackend(
  'scrapling-parse',
  async function* (_prompt, _mode, _model, opts) {
    const signal = opts?.signal
    yield* streamThought(
      'parsing the page: locate the heading, describe it, then render Markdown…',
      { signal },
    )
    yield* streamFcall({
      functionId: 'scrapling::find-by-text',
      input: scraplingFindByText.input,
      output: scraplingFindByText.output,
      waitMs: 300,
      signal,
    })
    yield* streamFcall({
      functionId: 'scrapling::describe',
      input: scraplingDescribe.input,
      output: scraplingDescribe.output,
      waitMs: 350,
      signal,
    })
    yield* streamFcall({
      functionId: 'scrapling::to-markdown',
      input: scraplingToMarkdown.input,
      output: scraplingToMarkdown.output,
      waitMs: 400,
      signal,
    })
    yield* streamAssistant(
      'found the `<h1>` (selector `body > h1`) and rendered the page to Markdown.',
      { signal },
    )
  },
)
