import { watch } from 'node:fs'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { dirname, isAbsolute, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { parseArgs } from 'node:util'
import { uiPage, uiStyles } from 'virtual:slides-ui-assets'
import { registerWorker } from 'iii-sdk'
import { auditDeck } from './audit.js'
import { CaptureUnavailable, captureOverview, captureSlides, measureSlides } from './capture.js'
import { type Config, expandHome, loadConfig } from './config.js'
import { bindConfigTrigger, fetchRuntime, registerSlidesConfig } from './configuration.js'
import {
  applyOperations,
  type BlockEdit,
  insertBlock,
  OPERATIONS,
  removeBlock,
  reorderBlocks,
  updateBlock,
  withSlide,
} from './edits.js'
import {
  type Deck,
  deckToMarkdown,
  insertSlide,
  isDeck,
  mergeSlide,
  newId,
  normalizeMotion,
  normalizeOverrides,
  normalizeSlide,
  normalizeSlides,
  reorderSlides,
  type Slide,
  slideIndex,
  slidesFromMarkdown,
  summarize,
  text,
} from './model.js'
import { draftDeck } from './outline.js'
import { renderDeckHtml } from './render-html.js'
import { renderDeckPdf } from './render-pdf.js'
import { renderDeckPptx } from './render-pptx.js'
import { rasterPdf, rasterPptx } from './render-raster.js'
import {
  array,
  blockSchema,
  boolean,
  deckResponse,
  deckSchema,
  deckSummarySchema,
  integer,
  motionSchema,
  nullableString,
  object,
  slideSchema,
  string,
  themeOverridesSchema,
  themeSchema,
} from './schemas.js'
import { DEFAULT_THEME, isThemeId, THEMES } from './themes.js'

const WORKER = 'slides'
const DECKS_SCOPE = 'slides_decks'
const CHANGED_TRIGGER = 'slides::changed'

const { values } = parseArgs({
  options: { config: { type: 'string', default: './config.yaml' }, url: { type: 'string' } },
  strict: false,
})

const seed = await loadConfig(String(values.config))
const url =
  (values.url ? String(values.url) : undefined) ?? process.env.III_URL ?? process.env.III_ENGINE_URL ?? seed.engine_url
const holder: { current: Config } = { current: { ...seed, engine_url: url } }

const iii = registerWorker(url, {
  workerName: WORKER,
  workerDescription: 'Slide decks: author, draft, render and export presentations, with a Console editor.',
})

function brand() {
  const { brand_accent, brand_footer } = holder.current
  return {
    ...(brand_accent ? { accent: brand_accent } : {}),
    ...(brand_footer ? { footer: brand_footer } : {}),
  }
}

async function stateGet<T>(key: string): Promise<T | null> {
  const value = await iii.trigger<Record<string, unknown>, T | null>({
    function_id: 'state::get',
    payload: { scope: DECKS_SCOPE, key },
    timeoutMs: 10_000,
  })
  return value ?? null
}

async function stateList<T>(): Promise<T[]> {
  const value = await iii.trigger<Record<string, unknown>, T[] | null>({
    function_id: 'state::list',
    payload: { scope: DECKS_SCOPE },
    timeoutMs: 10_000,
  })
  return Array.isArray(value) ? value : []
}

async function stateSet<T>(key: string, value: T): Promise<void> {
  await iii.trigger({ function_id: 'state::set', payload: { scope: DECKS_SCOPE, key, value }, timeoutMs: 10_000 })
}

async function stateDelete(key: string): Promise<void> {
  await iii.trigger({ function_id: 'state::delete', payload: { scope: DECKS_SCOPE, key }, timeoutMs: 10_000 })
}

type ChangeBinding = { id: string; function_id: string; namespace?: string; deck_id?: string }
const changeBindings = new Map<string, ChangeBinding>()

iii.registerTriggerType<{ deck_id?: string }>(
  {
    id: CHANGED_TRIGGER,
    description:
      'Fires when a deck is created, updated or deleted. Config: { deck_id? } to watch one deck; empty for all.',
  },
  {
    async registerTrigger({ id, function_id, namespace, config }) {
      changeBindings.set(id, {
        id,
        function_id,
        namespace,
        ...(text(config?.deck_id) ? { deck_id: text(config?.deck_id) } : {}),
      })
    },
    async unregisterTrigger({ id }) {
      changeBindings.delete(id)
    },
  },
)

async function emitChanged(
  kind: 'created' | 'updated' | 'deleted',
  deck: Pick<Deck, 'id' | 'revision'>,
): Promise<void> {
  const payload = { kind, deck_id: deck.id, revision: deck.revision, updated_at_ms: Date.now() }
  await Promise.all(
    [...changeBindings.values()]
      .filter((binding) => !binding.deck_id || binding.deck_id === deck.id)
      .map((binding) =>
        iii
          .trigger({
            function_id: binding.function_id,
            payload,
            timeoutMs: 10_000,
            ...(binding.namespace ? { namespace: binding.namespace } : {}),
          })
          .catch((error) =>
            console.error(`[${WORKER}] change delivery to ${binding.function_id} failed: ${String(error)}`),
          ),
      ),
  )
}

async function loadDeck(deckId: unknown): Promise<Deck> {
  const id = text(deckId)
  if (!id) throw new Error('INVALID_DECK: deck_id is required')
  const deck = await stateGet<Deck>(id)
  if (!isDeck(deck)) throw new Error(`DECK_NOT_FOUND: ${id}`)
  return deck
}

async function saveDeck(deck: Deck, kind: 'created' | 'updated'): Promise<Deck> {
  const next: Deck = { ...deck, revision: kind === 'created' ? 1 : deck.revision + 1, updated_at_ms: Date.now() }
  await stateSet(next.id, next)
  await emitChanged(kind, next)
  return next
}

function chooseTheme(candidate: unknown): string {
  if (isThemeId(candidate)) return candidate
  if (candidate !== undefined && candidate !== null && candidate !== '') {
    throw new Error(`INVALID_THEME: ${String(candidate)}; use one of ${THEMES.map((theme) => theme.id).join(', ')}`)
  }
  return isThemeId(holder.current.default_theme) ? holder.current.default_theme : DEFAULT_THEME
}

interface CreateInput {
  title?: string
  subtitle?: string
  author?: string
  theme?: string
  theme_overrides?: unknown
  motion?: unknown
  slides?: unknown
  markdown?: string
}

async function createDeck(input: CreateInput): Promise<Deck> {
  const fromMarkdown = text(input.markdown) ? slidesFromMarkdown(input.markdown as string) : null
  const slides = fromMarkdown ? fromMarkdown.slides : normalizeSlides(input.slides)
  const title =
    text(input.title) ?? fromMarkdown?.title ?? slides.find((slide) => slide.title)?.title ?? 'Untitled deck'
  const subtitle = text(input.subtitle) ?? fromMarkdown?.subtitle
  const now = Date.now()
  const deck: Deck = {
    id: newId('deck'),
    title,
    ...(subtitle ? { subtitle } : {}),
    ...(text(input.author) ? { author: text(input.author) } : {}),
    theme: chooseTheme(input.theme),
    ...(normalizeOverrides(input.theme_overrides)
      ? { theme_overrides: normalizeOverrides(input.theme_overrides) }
      : {}),
    ...(normalizeMotion(input.motion) ? { motion: normalizeMotion(input.motion) } : {}),
    slides: slides.length
      ? slides
      : [{ id: newId('slide'), layout: 'title', title, ...(subtitle ? { subtitle } : {}), blocks: [] }],
    revision: 0,
    created_at_ms: now,
    updated_at_ms: now,
  }
  return saveDeck(deck, 'created')
}

const createInputSchema = object(
  {
    title: nullableString,
    subtitle: nullableString,
    author: nullableString,
    theme: { type: ['string', 'null'], enum: [...THEMES.map((theme) => theme.id), null] },
    theme_overrides: themeOverridesSchema,
    motion: motionSchema,
    slides: array(slideSchema),
    markdown: {
      type: ['string', 'null'],
      description:
        'Alternative to slides: Markdown with slides separated by a line containing ---. # title, ## headings, - bullets, > quotes, ![alt](src) images, ``` code fences, <!-- layout: section --> and <!-- notes: ... --> directives.',
    },
  },
  [],
)

iii.registerFunction('slides::create', (input: CreateInput) => createDeck(input ?? {}).then((deck) => ({ deck })), {
  description:
    'Create and persist a deck from structured slides or Markdown. Returns the full deck with generated ids. Themes: midnight, paper, aurora, slate, sunrise, forest.',
  request_format: createInputSchema,
  response_format: deckResponse,
})

iii.registerFunction(
  'slides::list',
  async () => {
    const decks = (await stateList<Deck>()).filter(isDeck).map(summarize)
    decks.sort((a, b) => b.updated_at_ms - a.updated_at_ms)
    return { decks }
  },
  {
    description: 'List every stored deck as a summary (id, title, theme, slide count, revision), newest first.',
    request_format: object(),
    response_format: object({ decks: array(deckSummarySchema) }, ['decks']),
  },
)

iii.registerFunction('slides::get', async (input: { deck_id: string }) => ({ deck: await loadDeck(input?.deck_id) }), {
  description: 'Read one deck in full, including every slide, block and speaker note.',
  request_format: object({ deck_id: string }, ['deck_id']),
  response_format: deckResponse,
})

iii.registerFunction(
  'slides::markdown',
  async (input: { deck_id: string }) => {
    const deck = await loadDeck(input?.deck_id)
    return { deck_id: deck.id, markdown: deckToMarkdown(deck) }
  },
  {
    description: 'Serialize a deck to the Markdown dialect slides::create accepts; the compact way to review a deck.',
    request_format: object({ deck_id: string }, ['deck_id']),
    response_format: object({ deck_id: string, markdown: string }, ['deck_id', 'markdown']),
  },
)

interface UpdateInput extends CreateInput {
  deck_id: string
}

iii.registerFunction(
  'slides::update',
  async (input: UpdateInput) => {
    const deck = await loadDeck(input?.deck_id)
    const next: Deck = { ...deck }
    if (input.title !== undefined) next.title = text(input.title) ?? deck.title
    if (input.subtitle !== undefined) {
      if (text(input.subtitle)) next.subtitle = text(input.subtitle)
      else delete next.subtitle
    }
    if (input.author !== undefined) {
      if (text(input.author)) next.author = text(input.author)
      else delete next.author
    }
    if (input.theme !== undefined) next.theme = chooseTheme(input.theme)
    if (input.theme_overrides !== undefined) {
      const overrides = normalizeOverrides(input.theme_overrides)
      if (overrides) next.theme_overrides = overrides
      else delete next.theme_overrides
    }
    if (input.motion !== undefined) {
      const motion = normalizeMotion(input.motion)
      if (motion) next.motion = motion
      else delete next.motion
    }
    if (input.markdown !== undefined && text(input.markdown))
      next.slides = slidesFromMarkdown(input.markdown as string).slides
    else if (input.slides !== undefined) next.slides = normalizeSlides(input.slides)
    return { deck: await saveDeck(next, 'updated') }
  },
  {
    description:
      'Update deck metadata, theme, overrides, motion (transition, reveal), or replace the whole slide array (or all slides from Markdown). Omitted fields are kept; empty strings clear optional fields.',
    request_format: object({ deck_id: string, ...createInputSchema.properties }, ['deck_id']),
    response_format: deckResponse,
  },
)

function blockResponse(extra: Record<string, unknown> = {}, required: string[] = []) {
  return object({ deck: deckSchema, ...extra }, ['deck', ...required])
}

async function editSlide(deckId: unknown, slideId: unknown, edit: (slide: Slide) => Slide): Promise<Deck> {
  const deck = await loadDeck(deckId)
  const id = text(slideId)
  if (!id) throw new Error('INVALID_SLIDE: slide_id is required')
  const slide = deck.slides[slideIndex(deck, id)]
  return saveDeck(withSlide(deck, id, edit(slide)), 'updated')
}

iii.registerFunction(
  'slides::block::insert',
  async (input: { deck_id: string; slide_id: string } & BlockEdit) => {
    let blockId = ''
    const deck = await editSlide(input?.deck_id, input?.slide_id, (slide) => {
      const result = insertBlock(slide, input)
      blockId = result.block.id
      return result.slide
    })
    return { deck, block_id: blockId }
  },
  {
    description:
      'Insert one block into a slide at index (default: end) or after after_block_id, without resending the slide. Returns the deck and the new block id.',
    request_format: object(
      { deck_id: string, slide_id: string, block: blockSchema, index: integer, after_block_id: nullableString },
      ['deck_id', 'slide_id', 'block'],
    ),
    response_format: blockResponse({ block_id: string }, ['block_id']),
  },
)

iii.registerFunction(
  'slides::block::update',
  async (input: { deck_id: string; slide_id: string; block_id: string; block: unknown }) => ({
    deck: await editSlide(input?.deck_id, input?.slide_id, (slide) => updateBlock(slide, input)),
  }),
  {
    description:
      'Patch one block by id, keeping its id and position. Fields merge into the block; pass a different type to replace it (diagram nodes, table rows, chart series and captions change without resending the slide).',
    request_format: object({ deck_id: string, slide_id: string, block_id: string, block: blockSchema }, [
      'deck_id',
      'slide_id',
      'block_id',
      'block',
    ]),
    response_format: deckResponse,
  },
)

iii.registerFunction(
  'slides::block::remove',
  async (input: { deck_id: string; slide_id: string; block_id: string }) => ({
    deck: await editSlide(input?.deck_id, input?.slide_id, (slide) => removeBlock(slide, input)),
  }),
  {
    description: 'Remove one block by id.',
    request_format: object({ deck_id: string, slide_id: string, block_id: string }, [
      'deck_id',
      'slide_id',
      'block_id',
    ]),
    response_format: deckResponse,
  },
)

iii.registerFunction(
  'slides::block::reorder',
  async (input: { deck_id: string; slide_id: string; block_ids: string[] }) => ({
    deck: await editSlide(input?.deck_id, input?.slide_id, (slide) => reorderBlocks(slide, input?.block_ids)),
  }),
  {
    description:
      'Reorder the blocks of a slide. block_ids lists ids in the new order; omitted blocks keep their order at the end.',
    request_format: object({ deck_id: string, slide_id: string, block_ids: array(string) }, [
      'deck_id',
      'slide_id',
      'block_ids',
    ]),
    response_format: deckResponse,
  },
)

const operationSchema = object(
  {
    op: { type: 'string', enum: [...OPERATIONS] },
    slide_id: nullableString,
    slide: slideSchema,
    index: integer,
    after_slide_id: nullableString,
    slide_ids: array(string),
    block_id: nullableString,
    block: blockSchema,
    after_block_id: nullableString,
    block_ids: array(string),
  },
  ['op'],
  {
    description:
      'One operation. slide.insert uses slide, index or after_slide_id; slide.update/slide.remove use slide_id (and slide); slide.reorder uses slide_ids; block.insert uses slide_id, block, index or after_block_id; block.update/block.remove use slide_id and block_id (and block); block.reorder uses slide_id and block_ids.',
  },
)

iii.registerFunction(
  'slides::apply',
  async (input: { deck_id: string; ops: unknown; expect_revision?: number }) => {
    const deck = await loadDeck(input?.deck_id)
    if (
      input.expect_revision !== undefined &&
      input.expect_revision !== null &&
      input.expect_revision !== deck.revision
    )
      throw new Error(`REVISION_CONFLICT: expected revision ${input.expect_revision}, deck is at ${deck.revision}`)
    const { deck: next, applied } = applyOperations(deck, input.ops)
    return { deck: await saveDeck(next, 'updated'), applied }
  },
  {
    description:
      'Apply many slide and block operations in one call and one revision increment; nothing is saved if any operation fails. expect_revision rejects the batch with REVISION_CONFLICT when the deck changed since it was read, so an agent and the editor do not overwrite each other. Returns the deck and the ids each operation touched or created.',
    request_format: object({ deck_id: string, ops: array(operationSchema), expect_revision: integer }, [
      'deck_id',
      'ops',
    ]),
    response_format: blockResponse(
      {
        applied: array(object({ op: string, slide_id: string, block_id: nullableString }, ['op', 'slide_id'])),
      },
      ['applied'],
    ),
  },
)

function captureHtml(deck: Deck): string {
  return renderDeckHtml(deck, brand(), { capture: true })
}

function resolveSlide(deck: Deck, slide: unknown): number {
  if (typeof slide === 'number') {
    if (!Number.isInteger(slide) || slide < 0 || slide >= deck.slides.length)
      throw new Error(`SLIDE_NOT_FOUND: index ${slide} is out of range 0..${deck.slides.length - 1}`)
    return slide
  }
  const id = text(slide)
  if (!id) throw new Error('INVALID_SLIDE: slide must be a slide id or a zero-based index')
  return slideIndex(deck, id)
}

const findingSchema = object(
  {
    code: string,
    severity: { type: 'string', enum: ['error', 'warning', 'info'] },
    message: string,
    block_id: nullableString,
  },
  ['code', 'severity', 'message'],
)

const slideAuditSchema = object(
  {
    slide_id: string,
    index: integer,
    title: nullableString,
    layout: string,
    word_count: integer,
    block_count: integer,
    measured: boolean,
    overflow: boolean,
    minimum_font_size: { type: ['number', 'null'] },
    empty_space_ratio: { type: ['number', 'null'] },
    collisions: array(object({ a: string, b: string }, ['a', 'b'])),
    clipped_labels: array(string),
    repeated_words: array(object({ word: string, count: integer }, ['word', 'count'])),
    findings: array(findingSchema),
  },
  [
    'slide_id',
    'index',
    'layout',
    'word_count',
    'block_count',
    'measured',
    'overflow',
    'collisions',
    'clipped_labels',
    'repeated_words',
    'findings',
  ],
)

iii.registerFunction(
  'slides::audit',
  async (input: { deck_id: string; measure?: boolean }) => {
    const deck = await loadDeck(input?.deck_id)
    if (input.measure === false) return auditDeck(deck)
    try {
      return auditDeck(deck, await measureSlides(iii, captureHtml(deck)))
    } catch (error) {
      return auditDeck(deck, undefined, error instanceof Error ? error.message : String(error))
    }
  },
  {
    description:
      'Machine-readable layout diagnostics per slide so an agent can fix a deck without looking at pixels: content checks (word count, bullets, notes, repeated words, ragged tables, thin charts) plus, when the browser worker is installed, measurements of the rendered slide (overflow after auto-fit, smallest font size, block collisions, clipped labels, empty-space ratio). measure: false skips the browser. measured: false with measurement_error tells you the layout checks did not run.',
    request_format: object({ deck_id: string, measure: boolean }, ['deck_id']),
    response_format: object(
      {
        deck_id: string,
        revision: integer,
        measured: boolean,
        measurement_error: nullableString,
        slides: array(slideAuditSchema),
        error_count: integer,
        warning_count: integer,
      },
      ['deck_id', 'revision', 'measured', 'slides', 'error_count', 'warning_count'],
    ),
  },
)

const imageSchema = object(
  {
    index: integer,
    slide_id: nullableString,
    content_type: string,
    width: integer,
    height: integer,
    data_base64: string,
  },
  ['index', 'content_type', 'width', 'height', 'data_base64'],
)

iii.registerFunction(
  'slides::snapshot',
  async (input: { deck_id: string; slide?: unknown; slides?: unknown[]; scale?: number; overview?: boolean }) => {
    const deck = await loadDeck(input?.deck_id)
    const html = captureHtml(deck)
    const scale = typeof input.scale === 'number' ? Math.max(0.25, Math.min(3, input.scale)) : 1
    if (input.overview) {
      const image = await captureOverview(iii, html, scale)
      return { deck_id: deck.id, revision: deck.revision, images: [{ ...image, slide_id: null }] }
    }
    const requested = Array.isArray(input.slides) ? input.slides : input.slide !== undefined ? [input.slide] : null
    const indexes = requested ? requested.map((slide) => resolveSlide(deck, slide)) : deck.slides.map((_, i) => i)
    const images = await captureSlides(iii, html, indexes, scale)
    return {
      deck_id: deck.id,
      revision: deck.revision,
      images: images.map((image) => ({ ...image, slide_id: deck.slides[image.index]?.id ?? null })),
    }
  },
  {
    description:
      'Render slides headlessly through the browser worker and return them as images (JPEG, base64), exactly as the presentation and exports look. slide takes one slide id or zero-based index, slides a list; omit both for every slide. scale multiplies the 1600x900 canvas (default 1). overview: true returns one contact sheet of the whole deck instead. Requires the browser worker; fails with CAPTURE_UNAVAILABLE otherwise.',
    request_format: object(
      {
        deck_id: string,
        slide: { type: ['string', 'integer', 'null'], description: 'Slide id or zero-based index' },
        slides: array({ type: ['string', 'integer'] }),
        scale: { type: 'number', minimum: 0.25, maximum: 3 },
        overview: boolean,
      },
      ['deck_id'],
    ),
    response_format: object({ deck_id: string, revision: integer, images: array(imageSchema) }, [
      'deck_id',
      'revision',
      'images',
    ]),
  },
)
iii.registerFunction(
  'slides::delete',
  async (input: { deck_id: string }) => {
    const deck = await loadDeck(input?.deck_id)
    await stateDelete(deck.id)

    await emitChanged('deleted', deck)
    return { deleted: true, deck_id: deck.id }
  },
  {
    description: 'Delete a deck permanently.',
    request_format: object({ deck_id: string }, ['deck_id']),
    response_format: object({ deleted: boolean, deck_id: string }, ['deleted', 'deck_id']),
  },
)

iii.registerFunction(
  'slides::slide::insert',
  async (input: { deck_id: string; slide?: unknown; index?: number; after_slide_id?: string }) => {
    const deck = await loadDeck(input?.deck_id)
    const slide = normalizeSlide(input.slide ?? { layout: 'content', title: 'New slide' })
    slide.id = newId('slide')
    const index = text(input.after_slide_id) ? slideIndex(deck, input.after_slide_id as string) + 1 : input.index
    const next = await saveDeck({ ...deck, slides: insertSlide(deck, slide, index) }, 'updated')
    return { deck: next, slide_id: slide.id }
  },
  {
    description:
      'Insert one slide at index (default: end) or after after_slide_id. Returns the deck and the new slide id.',
    request_format: object({ deck_id: string, slide: slideSchema, index: integer, after_slide_id: nullableString }, [
      'deck_id',
    ]),
    response_format: object({ deck: deckSchema, slide_id: string }, ['deck', 'slide_id']),
  },
)

iii.registerFunction(
  'slides::slide::update',
  async (input: { deck_id: string; slide_id: string; slide: unknown }) => {
    const deck = await loadDeck(input?.deck_id)
    const index = slideIndex(deck, text(input.slide_id) ?? '')
    const slides = [...deck.slides]
    slides[index] = mergeSlide(slides[index], input.slide ?? {})
    return { deck: await saveDeck({ ...deck, slides }, 'updated') }
  },
  {
    description:
      'Patch one slide: any of layout, title, subtitle, blocks (replaces all blocks), notes, background. Omitted fields are kept.',
    request_format: object({ deck_id: string, slide_id: string, slide: slideSchema }, ['deck_id', 'slide_id', 'slide']),
    response_format: deckResponse,
  },
)

iii.registerFunction(
  'slides::slide::remove',
  async (input: { deck_id: string; slide_id: string }) => {
    const deck = await loadDeck(input?.deck_id)
    const index = slideIndex(deck, text(input.slide_id) ?? '')
    const slides = deck.slides.filter((_, i) => i !== index)
    return { deck: await saveDeck({ ...deck, slides }, 'updated') }
  },
  {
    description: 'Remove one slide by id.',
    request_format: object({ deck_id: string, slide_id: string }, ['deck_id', 'slide_id']),
    response_format: deckResponse,
  },
)

iii.registerFunction(
  'slides::slide::reorder',
  async (input: { deck_id: string; slide_ids: string[] }) => {
    const deck = await loadDeck(input?.deck_id)
    if (!Array.isArray(input.slide_ids)) throw new Error('INVALID_ORDER: slide_ids must be an array')
    return { deck: await saveDeck({ ...deck, slides: reorderSlides(deck, input.slide_ids) }, 'updated') }
  },
  {
    description:
      'Reorder slides. slide_ids lists ids in the new order; ids not listed keep their relative order at the end.',
    request_format: object({ deck_id: string, slide_ids: array(string) }, ['deck_id', 'slide_ids']),
    response_format: deckResponse,
  },
)

iii.registerFunction(
  'slides::render',
  async (input: { deck_id: string; editing?: boolean }) => {
    const deck = await loadDeck(input?.deck_id)
    return {
      deck_id: deck.id,
      revision: deck.revision,
      content_type: 'text/html',
      html: renderDeckHtml(deck, brand(), { editing: input.editing === true }),
    }
  },
  {
    description:
      'Render a deck to a self-contained HTML presentation (keyboard navigation, speaker notes with n, print to PDF). editing: true renders the Console editing view (no motion, block ids, click-to-select).',
    request_format: object({ deck_id: string, editing: boolean }, ['deck_id']),
    response_format: object({ deck_id: string, revision: integer, content_type: string, html: string }, [
      'deck_id',
      'revision',
      'content_type',
      'html',
    ]),
  },
)

iii.registerFunction(
  'slides::preview',
  async (input: { deck: CreateInput & { id?: string }; editing?: boolean }) => {
    const raw = input?.deck ?? {}
    const now = Date.now()
    const slides = text(raw.markdown) ? slidesFromMarkdown(raw.markdown as string).slides : normalizeSlides(raw.slides)
    const deck: Deck = {
      id: text(raw.id) ?? 'preview',
      title: text(raw.title) ?? 'Untitled deck',
      ...(text(raw.subtitle) ? { subtitle: text(raw.subtitle) } : {}),
      ...(text(raw.author) ? { author: text(raw.author) } : {}),
      theme: chooseTheme(raw.theme),
      ...(normalizeOverrides(raw.theme_overrides) ? { theme_overrides: normalizeOverrides(raw.theme_overrides) } : {}),
      ...(normalizeMotion(raw.motion) ? { motion: normalizeMotion(raw.motion) } : {}),
      slides,
      revision: 0,
      created_at_ms: now,
      updated_at_ms: now,
    }
    return { content_type: 'text/html', html: renderDeckHtml(deck, brand(), { editing: input.editing === true }) }
  },
  {
    description:
      'Render a deck that is not saved (the same payload slides::create accepts, plus optional id) to HTML. The Console editor uses it for live preview; editing: true adds block ids and click-to-select.',
    request_format: object(
      { deck: object({ id: nullableString, ...createInputSchema.properties }), editing: boolean },
      ['deck'],
    ),
    response_format: object({ content_type: string, html: string }, ['content_type', 'html']),
  },
)

type ExportFormat = 'html' | 'pdf' | 'pptx'
const EXPORT_TYPES: Record<ExportFormat, string> = {
  html: 'text/html',
  pdf: 'application/pdf',
  pptx: 'application/vnd.openxmlformats-officedocument.presentationml.presentation',
}

function slug(value: string): string {
  return (
    value
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, '-')
      .replace(/^-+|-+$/g, '')
      .slice(0, 60) || 'deck'
  )
}

type ExportEngine = 'browser' | 'native'

async function renderNative(deck: Deck, format: ExportFormat): Promise<Buffer> {
  if (format === 'pdf') return Buffer.from(await renderDeckPdf(deck, brand()))
  return renderDeckPptx(deck, brand())
}

async function renderExport(
  deck: Deck,
  format: ExportFormat,
  requested: ExportEngine | undefined,
): Promise<{ bytes: Buffer; engine: ExportEngine; fallback_reason?: string }> {
  if (format === 'html') return { bytes: Buffer.from(renderDeckHtml(deck, brand()), 'utf8'), engine: 'native' }
  if (requested === 'native') return { bytes: await renderNative(deck, format), engine: 'native' }
  try {
    const images = await captureSlides(
      iii,
      captureHtml(deck),
      deck.slides.map((_, i) => i),
    )
    const bytes = format === 'pdf' ? Buffer.from(await rasterPdf(deck, images)) : await rasterPptx(deck, images)
    return { bytes, engine: 'browser' }
  } catch (error) {
    if (requested === 'browser') throw error
    const reason = error instanceof Error ? error.message : String(error)
    if (!(error instanceof CaptureUnavailable))
      console.error(`[${WORKER}] browser export failed, using the native renderer: ${reason}`)
    return { bytes: await renderNative(deck, format), engine: 'native', fallback_reason: reason }
  }
}

iii.registerFunction(
  'slides::export',
  async (input: { deck_id: string; format: ExportFormat; inline?: boolean; path?: string; engine?: ExportEngine }) => {
    const deck = await loadDeck(input?.deck_id)
    const format = input.format
    if (!(format in EXPORT_TYPES)) throw new Error('INVALID_FORMAT: format must be html, pdf or pptx')
    if (input.engine !== undefined && input.engine !== null && input.engine !== 'browser' && input.engine !== 'native')
      throw new Error('INVALID_ENGINE: engine must be browser or native')
    const { bytes, engine, fallback_reason } = await renderExport(deck, format, input.engine ?? undefined)
    let path: string
    if (text(input.path)) {
      if (!isAbsolute(input.path as string)) throw new Error('INVALID_PATH: path must be absolute')
      path = resolve(input.path as string)
    } else path = join(expandHome(holder.current.output_dir), `${slug(deck.title)}-${deck.id.slice(-8)}.${format}`)
    await mkdir(dirname(path), { recursive: true })
    await writeFile(path, bytes)
    return {
      deck_id: deck.id,
      revision: deck.revision,
      format,
      content_type: EXPORT_TYPES[format],
      path,
      size: bytes.byteLength,
      engine,
      ...(fallback_reason ? { fallback_reason } : {}),
      ...(input.inline ? { data_base64: bytes.toString('base64') } : {}),
    }
  },
  {
    description:
      'Export a deck as html, pdf or pptx. Writes the file under output_dir (or the absolute path given) and returns its path; inline: true also returns the bytes as data_base64. pdf and pptx render each slide headlessly through the browser worker (pixel-identical to the presentation, theme fonts, diagrams and tables included, notes kept in pptx) and fall back to the native vector renderers when the browser worker is missing; engine forces browser or native.',
    request_format: object(
      {
        deck_id: string,
        format: { type: 'string', enum: ['html', 'pdf', 'pptx'] },
        inline: boolean,
        path: nullableString,
        engine: { type: ['string', 'null'], enum: ['browser', 'native', null] },
      },
      ['deck_id', 'format'],
    ),
    response_format: object(
      {
        deck_id: string,
        revision: integer,
        format: string,
        content_type: string,
        path: string,
        size: integer,
        engine: { type: 'string', enum: ['browser', 'native'] },
        fallback_reason: nullableString,
        data_base64: nullableString,
      },
      ['deck_id', 'revision', 'format', 'content_type', 'path', 'size', 'engine'],
    ),
  },
)

iii.registerFunction(
  'slides::themes::list',
  async () => ({
    default_theme: chooseTheme(undefined),
    themes: THEMES.map(({ id, name, description, dark, colors, fonts }) => ({
      id,
      name,
      description,
      dark,
      colors,
      fonts,
    })),
  }),
  {
    description: 'List the built-in themes with their palette and fonts, plus the configured default.',
    request_format: object(),
    response_format: object({ default_theme: string, themes: array(themeSchema) }, ['default_theme', 'themes']),
  },
)

interface OutlineRequest {
  topic: string
  audience?: string
  slide_count?: number
  tone?: string
  context?: string
  theme?: string
  author?: string
  model?: string
  provider?: string
  deck_id?: string
}

iii.registerFunction(
  'slides::outline',
  async (input: OutlineRequest) => {
    const theme =
      input?.theme !== undefined && input.theme !== null && input.theme !== '' ? chooseTheme(input.theme) : undefined
    const draft = await draftDeck(iii, holder.current, { ...input, ...(theme ? { theme } : {}) })
    if (text(input.deck_id)) {
      const existing = await loadDeck(input.deck_id)
      const deck = await saveDeck(
        {
          ...existing,
          title: draft.title,
          ...(draft.subtitle ? { subtitle: draft.subtitle } : {}),
          theme: theme ?? draft.theme ?? existing.theme,
          slides: draft.slides,
        },
        'updated',
      )
      return { deck, model: draft.model }
    }
    const deck = await createDeck({
      title: draft.title,
      subtitle: draft.subtitle,
      author: input.author,
      theme: theme ?? draft.theme,
      slides: draft.slides,
    })
    return { deck, model: draft.model }
  },
  {
    description:
      "Draft a complete deck from a brief with the LLM router (titles as takeaways, varied layouts, metrics, speaker notes) and persist it. Pass deck_id to replace an existing deck's slides. model defaults to the configured default_model, then the first chat model in the router catalog.",
    request_format: object(
      {
        topic: string,
        audience: nullableString,
        slide_count: { type: 'integer', minimum: 1, maximum: 60 },
        tone: nullableString,
        context: {
          type: ['string', 'null'],
          description: 'Source material, facts, links and constraints the deck must respect',
        },
        theme: { type: ['string', 'null'], enum: [...THEMES.map((theme) => theme.id), null] },
        author: nullableString,
        model: nullableString,
        provider: nullableString,
        deck_id: nullableString,
      },
      ['topic'],
    ),
    response_format: object({ deck: deckSchema, model: string }, ['deck', 'model']),
  },
)

type UiAsset = { file: string; type: 'console:script' | 'console:style'; content_type: string; content: string }

const uiAssets: Record<string, UiAsset> = {
  'slides/page.js': { file: 'page.js', type: 'console:script', content_type: 'text/javascript', content: uiPage },
  'slides/styles.css': { file: 'styles.css', type: 'console:style', content_type: 'text/css', content: uiStyles },
}
const uiWatch = process.env.III_SLIDES_UI_WATCH
const uiWatchEnabled = Boolean(uiWatch)
const uiWatchDir =
  uiWatchEnabled && uiWatch !== '1' && uiWatch !== 'true'
    ? (uiWatch as string)
    : join(dirname(fileURLToPath(import.meta.url)), '..', '..', 'ui', 'dist')

async function uiContent(path: string) {
  const asset = uiAssets[path]
  if (!asset) throw new Error(`unknown ui asset: ${path}`)
  const content = uiWatchEnabled ? await readFile(join(uiWatchDir, asset.file), 'utf8') : asset.content
  return { content, content_type: asset.content_type }
}

iii.registerFunction('slides::ui-content', (input: { path: string }) => uiContent(input.path), {
  description: 'Serve the injectable Slides Console page assets.',
  metadata: { internal: true },
  request_format: object({ path: string }, ['path']),
  response_format: object({ content: string, content_type: string }, ['content', 'content_type']),
})

function registerUiAsset(path: string) {
  return iii.registerTrigger({ type: uiAssets[path].type, function_id: 'slides::ui-content', config: { path } })
}

const uiTriggers = new Map(Object.keys(uiAssets).map((path) => [path, registerUiAsset(path)]))

if (uiWatchEnabled) {
  const pending = new Map<string, NodeJS.Timeout>()
  watch(uiWatchDir, (_event, file) => {
    const path = Object.keys(uiAssets).find((key) => uiAssets[key].file === file)
    if (!path) return
    clearTimeout(pending.get(path))
    pending.set(
      path,
      setTimeout(() => {
        const previous = uiTriggers.get(path)
        uiTriggers.set(path, registerUiAsset(path))
        previous?.unregister()
        console.error(`[${WORKER}] reloaded ui asset ${path}`)
      }, 150),
    )
  })
  console.error(`[${WORKER}] serving ui assets from ${uiWatchDir}`)
}

bindConfigTrigger(iii, async () => {
  const runtime = await fetchRuntime(iii)
  if (runtime) holder.current = { engine_url: url, ...runtime }
})

try {
  await registerSlidesConfig(iii, holder.current)
} catch (err) {
  console.warn(`configuration::register failed; continuing with the seed: ${String(err)}`)
}
const runtime = await fetchRuntime(iii)
if (runtime) holder.current = { engine_url: url, ...runtime }

console.log(`${WORKER} worker connected`)

const shutdown = async () => {
  await iii.shutdown()
  process.exit(0)
}
process.on('SIGINT', shutdown)
process.on('SIGTERM', shutdown)
