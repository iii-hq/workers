import {
  Button,
  IconButton,
  Input,
  Select,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
  uiClasses,
} from '@iii-dev/console-ui'
import type { ReactNode } from 'react'
import { ArrowDown, ArrowUp, Plus, Trash } from './icons'
import {
  BLOCK_LABEL,
  BLOCK_TYPES,
  type Block,
  type BlockType,
  type Deck,
  defaultBlock,
  LAYOUT_LABEL,
  LAYOUTS,
  type Layout,
  type Slide,
  type Theme,
} from './types'

function Field({ id, label, hint, children }: { id: string; label: string; hint?: string; children: ReactNode }) {
  return (
    <div className={uiClasses.field}>
      <label htmlFor={id} className={uiClasses.fieldLabel}>
        {label}
      </label>
      {children}
      {hint ? <span className={uiClasses.fieldDescription}>{hint}</span> : null}
    </div>
  )
}

function TextArea({
  id,
  value,
  rows = 3,
  placeholder,
  onChange,
  mono,
}: {
  id: string
  value: string
  rows?: number
  placeholder?: string
  onChange: (next: string) => void
  mono?: boolean
}) {
  return (
    <textarea
      id={id}
      className={`sl-textarea${mono ? ' sl-mono' : ''}`}
      rows={rows}
      value={value}
      placeholder={placeholder}
      onChange={(event) => onChange(event.target.value)}
    />
  )
}

export function DeckInspector({
  deck,
  themes,
  onChange,
}: {
  deck: Deck
  themes: Theme[]
  onChange: (patch: Partial<Deck>) => void
}) {
  const overrides = deck.theme_overrides ?? {}
  const setOverride = (key: keyof NonNullable<Deck['theme_overrides']>, value: string) => {
    const next = { ...overrides }
    if (value.trim()) next[key] = value.trim()
    else delete next[key]
    onChange({ theme_overrides: Object.keys(next).length ? next : undefined })
  }
  return (
    <div className="sl-inspector-body">
      <Field id="sl-deck-title" label="Deck title">
        <Input id="sl-deck-title" value={deck.title} onChange={(title) => onChange({ title })} />
      </Field>
      <Field id="sl-deck-subtitle" label="Subtitle">
        <Input
          id="sl-deck-subtitle"
          value={deck.subtitle ?? ''}
          onChange={(subtitle) => onChange({ subtitle: subtitle || undefined })}
        />
      </Field>
      <Field id="sl-deck-author" label="Author">
        <Input
          id="sl-deck-author"
          value={deck.author ?? ''}
          onChange={(author) => onChange({ author: author || undefined })}
        />
      </Field>
      <fieldset className={`${uiClasses.field} sl-fieldset`}>
        <legend className={uiClasses.fieldLabel}>Theme</legend>
        <div className="sl-theme-grid">
          {themes.map((theme) => (
            <button
              key={theme.id}
              type="button"
              aria-pressed={deck.theme === theme.id}
              className={`sl-theme-card${deck.theme === theme.id ? ' sl-theme-card-active' : ''}`}
              style={{ background: theme.colors.background, color: theme.colors.ink }}
              onClick={() => onChange({ theme: theme.id })}
              title={theme.description}
            >
              <span className="sl-theme-swatch" style={{ background: theme.colors.accent }} />
              <span className="sl-theme-name">{theme.name}</span>
            </button>
          ))}
        </div>
      </fieldset>
      <Field id="sl-deck-accent" label="Accent override" hint="Hex color, e.g. #ff6b57. Empty keeps the theme accent.">
        <div className="sl-color-row">
          <input
            type="color"
            aria-label="Pick accent color"
            className="sl-color-swatch"
            value={/^#[0-9a-f]{6}$/i.test(overrides.accent ?? '') ? (overrides.accent as string) : '#4f8cff'}
            onChange={(event) => setOverride('accent', event.target.value)}
          />
          <Input
            id="sl-deck-accent"
            value={overrides.accent ?? ''}
            placeholder="#4f8cff"
            onChange={(value) => setOverride('accent', value)}
          />
        </div>
      </Field>
      <Field id="sl-deck-footer" label="Footer text" hint="Shown bottom-left on every slide.">
        <Input id="sl-deck-footer" value={overrides.footer ?? ''} onChange={(value) => setOverride('footer', value)} />
      </Field>
    </div>
  )
}

function BlockEditor({
  block,
  index,
  count,
  twoColumn,
  selected,
  onSelect,
  onChange,
  onMove,
  onRemove,
}: {
  block: Block
  index: number
  count: number
  twoColumn: boolean
  selected: boolean
  onSelect: () => void
  onChange: (patch: Partial<Block>) => void
  onMove: (delta: -1 | 1) => void
  onRemove: () => void
}) {
  const id = `sl-block-${block.id}`
  let fields: ReactNode
  switch (block.type) {
    case 'heading':
      fields = <Input id={id} value={block.text ?? ''} onChange={(text) => onChange({ text })} placeholder="Heading" />
      break
    case 'text':
      fields = (
        <TextArea
          id={id}
          value={block.text ?? ''}
          onChange={(text) => onChange({ text })}
          placeholder="Text (use **bold** and `code`)"
        />
      )
      break
    case 'bullets':
      fields = (
        <TextArea
          id={id}
          rows={4}
          value={(block.items ?? []).join('\n')}
          onChange={(value) => onChange({ items: value.split('\n') })}
          placeholder="One bullet per line"
        />
      )
      break
    case 'image':
      fields = (
        <>
          <Input
            id={id}
            value={block.src ?? ''}
            onChange={(src) => onChange({ src })}
            placeholder="https://... or data:image/png;base64,..."
          />
          <Input
            aria-label="Alt text"
            value={block.alt ?? ''}
            onChange={(alt) => onChange({ alt })}
            placeholder="Alt text"
          />
          <Input
            aria-label="Caption"
            value={block.caption ?? ''}
            onChange={(caption) => onChange({ caption })}
            placeholder="Caption"
          />
        </>
      )
      break
    case 'code':
      fields = (
        <>
          <Input
            aria-label="Language"
            value={block.language ?? ''}
            onChange={(language) => onChange({ language })}
            placeholder="Language (ts, py, sh...)"
          />
          <TextArea
            id={id}
            rows={6}
            mono
            value={block.code ?? ''}
            onChange={(code) => onChange({ code })}
            placeholder="Code"
          />
        </>
      )
      break
    case 'quote':
      fields = (
        <>
          <TextArea id={id} value={block.text ?? ''} onChange={(text) => onChange({ text })} placeholder="Quote" />
          <Input
            aria-label="Attribution"
            value={block.attribution ?? ''}
            onChange={(attribution) => onChange({ attribution })}
            placeholder="Attribution"
          />
        </>
      )
      break
    case 'metric':
      fields = (
        <>
          <Input id={id} value={block.value ?? ''} onChange={(value) => onChange({ value })} placeholder="42%" />
          <Input
            aria-label="Label"
            value={block.label ?? ''}
            onChange={(label) => onChange({ label })}
            placeholder="What the number means"
          />
        </>
      )
      break
  }
  return (
    <div className={`sl-block-editor${selected ? ' sl-block-editor-active' : ''}`} data-block-id={block.id}>
      <div className="sl-block-editor-head">
        <button type="button" className="sl-block-editor-title" onClick={onSelect}>
          {BLOCK_LABEL[block.type]}
        </button>
        {twoColumn ? (
          <Select
            aria-label="Column"
            appearance="inline"
            value={block.column ?? (index % 2 === 0 ? 'left' : 'right')}
            options={[
              { value: 'left', label: 'Left' },
              { value: 'right', label: 'Right' },
            ]}
            onChange={(column) => onChange({ column: column as 'left' | 'right' })}
          />
        ) : null}
        <IconButton label="Move block up" variant="ghost" disabled={index === 0} onClick={() => onMove(-1)}>
          <ArrowUp />
        </IconButton>
        <IconButton label="Move block down" variant="ghost" disabled={index === count - 1} onClick={() => onMove(1)}>
          <ArrowDown />
        </IconButton>
        <IconButton label="Remove block" variant="ghost" onClick={onRemove}>
          <Trash />
        </IconButton>
      </div>
      <div className="sl-block-editor-fields">{fields}</div>
    </div>
  )
}

export function SlideInspector({
  slide,
  selectedBlockId,
  onSelectBlock,
  onChange,
}: {
  slide: Slide
  selectedBlockId: string | null
  onSelectBlock: (id: string | null) => void
  onChange: (patch: Partial<Slide>) => void
}) {
  const twoColumn = slide.layout === 'two-column'
  const updateBlock = (id: string, patch: Partial<Block>) =>
    onChange({ blocks: slide.blocks.map((block) => (block.id === id ? { ...block, ...patch } : block)) })
  const moveBlock = (id: string, delta: -1 | 1) => {
    const index = slide.blocks.findIndex((block) => block.id === id)
    const target = index + delta
    if (index < 0 || target < 0 || target >= slide.blocks.length) return
    const blocks = [...slide.blocks]
    ;[blocks[index], blocks[target]] = [blocks[target], blocks[index]]
    onChange({ blocks })
  }
  const addBlock = (type: BlockType) => {
    const block = defaultBlock(type)
    if (twoColumn) block.column = slide.blocks.length % 2 === 0 ? 'left' : 'right'
    onChange({ blocks: [...slide.blocks, block] })
    onSelectBlock(block.id)
  }
  return (
    <div className="sl-inspector-body">
      <Field id="sl-slide-layout" label="Layout">
        <Select
          id="sl-slide-layout"
          value={slide.layout}
          options={LAYOUTS.map((layout) => ({ value: layout, label: LAYOUT_LABEL[layout] }))}
          onChange={(layout) => onChange({ layout: layout as Layout })}
        />
      </Field>
      {slide.layout !== 'blank' ? (
        <>
          <Field id="sl-slide-title" label="Title" hint="State the takeaway as a sentence.">
            <TextArea
              id="sl-slide-title"
              rows={2}
              value={slide.title ?? ''}
              onChange={(title) => onChange({ title })}
            />
          </Field>
          <Field id="sl-slide-subtitle" label="Subtitle">
            <Input
              id="sl-slide-subtitle"
              value={slide.subtitle ?? ''}
              onChange={(subtitle) => onChange({ subtitle })}
            />
          </Field>
        </>
      ) : null}
      <div className="sl-blocks-head">
        <span className={uiClasses.fieldLabel}>Blocks</span>
        <Select
          aria-label="Add block"
          appearance="inline"
          value={undefined}
          placeholder="Add block"
          options={BLOCK_TYPES.map((type) => ({ value: type, label: BLOCK_LABEL[type] }))}
          onChange={(type) => addBlock(type as BlockType)}
        />
      </div>
      {slide.blocks.length === 0 ? <p className="sl-hint">No blocks yet. Add a metric, bullets or a quote.</p> : null}
      {slide.blocks.map((block, index) => (
        <BlockEditor
          key={block.id}
          block={block}
          index={index}
          count={slide.blocks.length}
          twoColumn={twoColumn}
          selected={selectedBlockId === block.id}
          onSelect={() => onSelectBlock(block.id)}
          onChange={(patch) => updateBlock(block.id, patch)}
          onMove={(delta) => moveBlock(block.id, delta)}
          onRemove={() => onChange({ blocks: slide.blocks.filter((candidate) => candidate.id !== block.id) })}
        />
      ))}
      <Field id="sl-slide-notes" label="Speaker notes" hint="Press n while presenting to show them.">
        <TextArea id="sl-slide-notes" rows={4} value={slide.notes ?? ''} onChange={(notes) => onChange({ notes })} />
      </Field>
      <Field id="sl-slide-background" label="Background" hint="Hex color or image URL; empty uses the theme.">
        <Input
          id="sl-slide-background"
          value={slide.background ?? ''}
          onChange={(background) => onChange({ background })}
          placeholder="#101010 or https://..."
        />
      </Field>
    </div>
  )
}

export function Inspector({
  deck,
  slide,
  themes,
  tab,
  onTabChange,
  selectedBlockId,
  onSelectBlock,
  onChangeDeck,
  onChangeSlide,
}: {
  deck: Deck
  slide: Slide | null
  themes: Theme[]
  tab: 'slide' | 'deck'
  onTabChange: (tab: 'slide' | 'deck') => void
  selectedBlockId: string | null
  onSelectBlock: (id: string | null) => void
  onChangeDeck: (patch: Partial<Deck>) => void
  onChangeSlide: (patch: Partial<Slide>) => void
}) {
  return (
    <aside className="sl-inspector" aria-label="Inspector">
      <Tabs value={tab} onValueChange={(value) => onTabChange(value as 'slide' | 'deck')} className="sl-inspector-tabs">
        <TabsList variant="line">
          <TabsTrigger value="slide">Slide</TabsTrigger>
          <TabsTrigger value="deck">Deck</TabsTrigger>
        </TabsList>
        <TabsContent value="slide">
          {slide ? (
            <SlideInspector
              slide={slide}
              selectedBlockId={selectedBlockId}
              onSelectBlock={onSelectBlock}
              onChange={onChangeSlide}
            />
          ) : (
            <p className="sl-hint">Select a slide.</p>
          )}
        </TabsContent>
        <TabsContent value="deck">
          <DeckInspector deck={deck} themes={themes} onChange={onChangeDeck} />
        </TabsContent>
      </Tabs>
    </aside>
  )
}

export function AddSlideMenu({ onAdd }: { onAdd: (layout: Layout) => void }) {
  return (
    <div className="sl-add-slide">
      <Select
        aria-label="Add slide"
        appearance="inline"
        value={undefined}
        placeholder="Add slide"
        options={LAYOUTS.map((layout) => ({ value: layout, label: LAYOUT_LABEL[layout] }))}
        onChange={(layout) => onAdd(layout as Layout)}
      />
      <Button variant="ghost" size="sm" onClick={() => onAdd('content')} aria-label="Add content slide">
        <Plus />
      </Button>
    </div>
  )
}
