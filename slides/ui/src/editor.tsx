import {
  Badge,
  Button,
  ConfirmDialog,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  IconButton,
  Input,
  Select,
  StatusPanel,
  uiClasses,
} from '@iii-dev/console-ui'
import { useCallback, useEffect, useRef, useState } from 'react'
import { type Api, downloadBytes } from './api'
import { ArrowDown, ArrowUp, Copy, Download, Notebook, Play, Trash, X } from './icons'
import { AddSlideMenu, Inspector } from './inspector'
import { SlideCanvas } from './slide-canvas'
import { type Block, type Deck, defaultSlide, describeError, type Layout, newId, type Slide, type Theme } from './types'

export function DraftDialog({
  open,
  themes,
  busy,
  error,
  onOpenChange,
  onDraft,
}: {
  open: boolean
  themes: Theme[]
  busy: boolean
  error: string | null
  onOpenChange: (open: boolean) => void
  onDraft: (input: Record<string, unknown>) => void
}) {
  const [topic, setTopic] = useState('')
  const [audience, setAudience] = useState('')
  const [count, setCount] = useState('10')
  const [tone, setTone] = useState('')
  const [context, setContext] = useState('')
  const [theme, setTheme] = useState<string | undefined>(undefined)
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sl-dialog">
        <DialogTitle>Draft a deck</DialogTitle>
        <DialogDescription>
          Describe the talk. The router drafts titles as takeaways, varied layouts, metrics and speaker notes; you edit
          from there.
        </DialogDescription>
        <form
          className="sl-dialog-form"
          onSubmit={(event) => {
            event.preventDefault()
            if (!topic.trim() || busy) return
            onDraft({
              topic: topic.trim(),
              ...(audience.trim() ? { audience: audience.trim() } : {}),
              ...(tone.trim() ? { tone: tone.trim() } : {}),
              ...(context.trim() ? { context: context.trim() } : {}),
              ...(theme ? { theme } : {}),
              slide_count: Math.max(1, Math.min(60, Number.parseInt(count, 10) || 10)),
            })
          }}
        >
          <div className={uiClasses.field}>
            <label htmlFor="sl-draft-topic" className={uiClasses.fieldLabel}>
              Topic
            </label>
            <Input
              id="sl-draft-topic"
              value={topic}
              onChange={setTopic}
              placeholder="Q3 results for the board"
              data-autofocus=""
            />
          </div>
          <div className="sl-dialog-row">
            <div className={uiClasses.field}>
              <label htmlFor="sl-draft-audience" className={uiClasses.fieldLabel}>
                Audience
              </label>
              <Input id="sl-draft-audience" value={audience} onChange={setAudience} placeholder="Board members" />
            </div>
            <div className={uiClasses.field}>
              <label htmlFor="sl-draft-count" className={uiClasses.fieldLabel}>
                Slides
              </label>
              <Input id="sl-draft-count" value={count} onChange={setCount} inputMode="numeric" />
            </div>
          </div>
          <div className="sl-dialog-row">
            <div className={uiClasses.field}>
              <label htmlFor="sl-draft-tone" className={uiClasses.fieldLabel}>
                Tone
              </label>
              <Input id="sl-draft-tone" value={tone} onChange={setTone} placeholder="Confident, plain" />
            </div>
            <div className={uiClasses.field}>
              <label htmlFor="sl-draft-theme" className={uiClasses.fieldLabel}>
                Theme
              </label>
              <Select
                id="sl-draft-theme"
                value={theme}
                placeholder="Let the model pick"
                allowEmpty
                emptyLabel="Let the model pick"
                onClear={() => setTheme(undefined)}
                options={themes.map((candidate) => ({
                  value: candidate.id,
                  label: candidate.name,
                  description: candidate.description,
                }))}
                onChange={setTheme}
              />
            </div>
          </div>
          <div className={uiClasses.field}>
            <label htmlFor="sl-draft-context" className={uiClasses.fieldLabel}>
              Source material
            </label>
            <textarea
              id="sl-draft-context"
              className="sl-textarea"
              rows={6}
              value={context}
              onChange={(event) => setContext(event.target.value)}
              placeholder="Facts, numbers, links and constraints the deck must respect."
            />
          </div>
          {error ? <StatusPanel variant="alert" headline="Drafting failed" detail={error} /> : null}
          <div className="sl-dialog-actions">
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" variant="primary" disabled={!topic.trim() || busy}>
              {busy ? 'Drafting\u2026' : 'Draft deck'}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  )
}

export function PresentOverlay({
  html,
  startIndex,
  onClose,
}: {
  html: string
  startIndex: number
  onClose: () => void
}) {
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])
  const src = html.replace(
    '<body class="deck">',
    `<body class="deck"><script>location.hash='#${startIndex + 1}'</script>`,
  )
  return (
    <div className="sl-present" role="dialog" aria-label="Presenting">
      <iframe
        className="sl-present-frame"
        title="Presentation"
        srcDoc={src}
        sandbox="allow-scripts allow-same-origin"
        data-autofocus=""
      />
      <div className="sl-present-bar">
        <span>Arrow keys to move, n for notes, f for fullscreen, Esc to leave</span>
        <IconButton label="Stop presenting" variant="ghost" onClick={onClose}>
          <X />
        </IconButton>
      </div>
    </div>
  )
}

export interface DeckEditorProps {
  api: Api
  deck: Deck
  themes: Theme[]
  saving: 'idle' | 'saving' | 'saved' | 'error'
  onChangeDeck: (patch: Partial<Deck>) => void
  onChangeSlides: (slides: Slide[]) => void
  onDeleteDeck: () => void
  onNotice: (kind: 'success' | 'error', text: string) => void
}

export function DeckEditor({
  api,
  deck,
  themes,
  saving,
  onChangeDeck,
  onChangeSlides,
  onDeleteDeck,
  onNotice,
}: DeckEditorProps) {
  const [current, setCurrent] = useState(0)
  const [selectedBlockId, setSelectedBlockId] = useState<string | null>(null)
  const [tab, setTab] = useState<'slide' | 'deck'>('slide')
  const [confirmSlide, setConfirmSlide] = useState(false)
  const [confirmDeck, setConfirmDeck] = useState(false)
  const [presenting, setPresenting] = useState<string | null>(null)
  const [exporting, setExporting] = useState<string | null>(null)
  const [inspectorOpen, setInspectorOpen] = useState(true)
  const [mode, setMode] = useState<'wide' | 'narrow' | 'compact'>('wide')
  const root = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const node = root.current
    if (!node) return
    let previous: 'wide' | 'narrow' | 'compact' = 'wide'
    const observer = new ResizeObserver(([entry]) => {
      const width = entry.contentRect.width
      const next = width < 640 ? 'compact' : width < 1000 ? 'narrow' : 'wide'
      if (next === previous) return
      previous = next
      setMode(next)
      setInspectorOpen(next === 'wide')
    })
    observer.observe(node)
    return () => observer.disconnect()
  }, [])

  const index = Math.min(current, Math.max(0, deck.slides.length - 1))
  const slide = deck.slides[index] ?? null
  const theme = themes.find((candidate) => candidate.id === deck.theme)

  useEffect(() => {
    setSelectedBlockId(null)
  }, [index])

  const updateSlide = useCallback(
    (slideId: string, patch: Partial<Slide>) =>
      onChangeSlides(
        deck.slides.map((candidate) => (candidate.id === slideId ? { ...candidate, ...patch } : candidate)),
      ),
    [deck.slides, onChangeSlides],
  )
  const updateBlock = (slideId: string, blockId: string, patch: Partial<Block>) => {
    const target = deck.slides.find((candidate) => candidate.id === slideId)
    if (!target) return
    updateSlide(slideId, {
      blocks: target.blocks.map((block) => (block.id === blockId ? { ...block, ...patch } : block)),
    })
  }
  const addSlide = (layout: Layout) => {
    const next = defaultSlide(layout)
    const slides = [...deck.slides]
    slides.splice(index + 1, 0, next)
    onChangeSlides(slides)
    setCurrent(index + 1)
    setTab('slide')
  }
  const duplicateSlide = () => {
    if (!slide) return
    const copy: Slide = {
      ...slide,
      id: newId('slide'),
      blocks: slide.blocks.map((block) => ({ ...block, id: newId('block') })),
    }
    const slides = [...deck.slides]
    slides.splice(index + 1, 0, copy)
    onChangeSlides(slides)
    setCurrent(index + 1)
  }
  const moveSlide = (delta: -1 | 1) => {
    const target = index + delta
    if (target < 0 || target >= deck.slides.length) return
    const slides = [...deck.slides]
    ;[slides[index], slides[target]] = [slides[target], slides[index]]
    onChangeSlides(slides)
    setCurrent(target)
  }
  const removeSlide = () => {
    if (!slide) return
    onChangeSlides(deck.slides.filter((candidate) => candidate.id !== slide.id))
    setCurrent(Math.max(0, index - 1))
    setConfirmSlide(false)
  }
  const present = async () => {
    try {
      const { html } = await api.render(deck.id)
      setPresenting(html)
    } catch (cause) {
      onNotice('error', describeError(cause))
    }
  }
  const exportAs = async (format: 'html' | 'pdf' | 'pptx') => {
    setExporting(format)
    try {
      const result = await api.exportDeck(deck.id, format)
      if (result.data_base64) {
        downloadBytes(result.data_base64, result.content_type, result.path.split('/').pop() ?? `deck.${format}`)
      }
      onNotice('success', `Exported ${format.toUpperCase()} to ${result.path}`)
    } catch (cause) {
      onNotice('error', describeError(cause))
    } finally {
      setExporting(null)
    }
  }

  return (
    <div ref={root} className={`sl-editor${mode === 'compact' ? ' sl-compact' : ''}`}>
      <div className="sl-toolbar">
        <div className="sl-toolbar-group">
          <AddSlideMenu onAdd={addSlide} />
          <IconButton label="Duplicate slide" variant="ghost" disabled={!slide} onClick={duplicateSlide}>
            <Copy />
          </IconButton>
          <IconButton label="Move slide up" variant="ghost" disabled={index === 0} onClick={() => moveSlide(-1)}>
            <ArrowUp />
          </IconButton>
          <IconButton
            label="Move slide down"
            variant="ghost"
            disabled={index >= deck.slides.length - 1}
            onClick={() => moveSlide(1)}
          >
            <ArrowDown />
          </IconButton>
          <IconButton
            label="Delete slide"
            variant="ghost"
            disabled={!slide || deck.slides.length <= 1}
            onClick={() => setConfirmSlide(true)}
          >
            <Trash />
          </IconButton>
        </div>
        <div className="sl-toolbar-group">
          <Badge variant={saving === 'error' ? 'alert' : saving === 'saving' ? 'accent' : 'default'}>
            {saving === 'saving'
              ? 'Saving'
              : saving === 'error'
                ? 'Save failed'
                : saving === 'saved'
                  ? 'Saved'
                  : `v${deck.revision}`}
          </Badge>
          <Button variant="ghost" size="sm" onClick={() => void present()} disabled={!deck.slides.length}>
            <Play /> <span className="sl-label">Present</span>
          </Button>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="ghost" size="sm" disabled={exporting !== null}>
                <Download />{' '}
                <span className="sl-label">{exporting ? `Exporting ${exporting.toUpperCase()}\u2026` : 'Export'}</span>
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuItem onSelect={() => void exportAs('pptx')}>PowerPoint (.pptx)</DropdownMenuItem>
              <DropdownMenuItem onSelect={() => void exportAs('pdf')}>PDF</DropdownMenuItem>
              <DropdownMenuItem onSelect={() => void exportAs('html')}>HTML presentation</DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
          <IconButton
            label={inspectorOpen ? 'Hide inspector' : 'Show inspector'}
            variant="ghost"
            onClick={() => setInspectorOpen((open) => !open)}
          >
            <Notebook />
          </IconButton>
          <IconButton label="Delete deck" variant="ghost" onClick={() => setConfirmDeck(true)}>
            <Trash />
          </IconButton>
        </div>
      </div>
      <div
        className={`sl-workspace${inspectorOpen ? ' sl-inspector-open' : ''}${mode !== 'wide' ? ' sl-narrow' : ''}${mode === 'compact' ? ' sl-compact' : ''}`}
      >
        <nav className="sl-rail" aria-label="Slides">
          {deck.slides.map((candidate, i) => (
            <button
              key={candidate.id}
              type="button"
              className={`sl-thumb${i === index ? ' sl-thumb-active' : ''}`}
              onClick={() => {
                setCurrent(i)
                setTab('slide')
              }}
              aria-label={`Slide ${i + 1}${candidate.title ? `: ${candidate.title}` : ''}`}
              aria-current={i === index ? 'true' : undefined}
            >
              <span className="sl-thumb-index">{i + 1}</span>
              <SlideCanvas
                slide={candidate}
                index={i}
                total={deck.slides.length}
                theme={theme}
                overrides={deck.theme_overrides}
                className="sl-thumb-canvas"
              />
            </button>
          ))}
        </nav>
        <div className="sl-stage">
          {slide ? (
            <SlideCanvas
              slide={slide}
              index={index}
              total={deck.slides.length}
              theme={theme}
              overrides={deck.theme_overrides}
              footer={deck.theme_overrides?.footer}
              author={deck.author}
              editable
              selectedBlockId={selectedBlockId}
              onSelectBlock={(id) => {
                setSelectedBlockId(id)
                if (id) setTab('slide')
              }}
              onChangeSlide={(patch) => updateSlide(slide.id, patch)}
              onChangeBlock={(blockId, patch) => updateBlock(slide.id, blockId, patch)}
              className="sl-stage-canvas"
            />
          ) : (
            <StatusPanel variant="info" headline="No slides" detail="Add a slide to start." />
          )}
          {slide?.notes ? (
            <div className="sl-notes-preview">
              <span className="sl-notes-label">Notes</span> {slide.notes}
            </div>
          ) : null}
        </div>
        <Inspector
          open={inspectorOpen}
          onClose={() => setInspectorOpen(false)}
          deck={deck}
          slide={slide}
          themes={themes}
          tab={tab}
          onTabChange={setTab}
          selectedBlockId={selectedBlockId}
          onSelectBlock={setSelectedBlockId}
          onChangeDeck={onChangeDeck}
          onChangeSlide={(patch) => slide && updateSlide(slide.id, patch)}
        />
      </div>
      <ConfirmDialog
        open={confirmSlide}
        onOpenChange={setConfirmSlide}
        title="Delete this slide?"
        description={
          slide?.title ? `"${slide.title}" will be removed from the deck.` : 'The slide will be removed from the deck.'
        }
        confirmLabel="Delete slide"
        onConfirm={removeSlide}
      />
      <ConfirmDialog
        open={confirmDeck}
        onOpenChange={setConfirmDeck}
        title="Delete this deck?"
        description={`"${deck.title}" and its ${deck.slides.length} slides will be deleted permanently.`}
        confirmLabel="Delete deck"
        onConfirm={() => {
          setConfirmDeck(false)
          onDeleteDeck()
        }}
      />
      {presenting ? <PresentOverlay html={presenting} startIndex={index} onClose={() => setPresenting(null)} /> : null}
    </div>
  )
}
