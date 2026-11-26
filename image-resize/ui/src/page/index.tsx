/** The image-resize page: pick an image, resize it over streams, compare it with the original, download it. */

import {
  Button,
  EmptyState,
  Eyebrow,
  type Host,
  IconButton,
  ImageThumbnailButton,
  ImageViewer,
  Input,
  type LiveAnnouncement,
  LiveRegion,
  MetaRow,
  type MetaRowItem,
  PageHeader,
  PageMain,
  type PageRenderProps,
  PageShell,
  Select,
  type SelectOption,
  SettingsField,
  SettingsList,
  SettingsSection,
  Skeleton,
  StatusPanel,
} from '@iii-dev/console-ui'
import { errorMessage, formatBytes } from '@iii-dev/console-ui/format'
import { Download, ImageIcon, Trash2 } from 'lucide-react'
import { type ReactNode, type Ref, useEffect, useReducer, useRef, useState } from 'react'
import {
  buildMetadata,
  canSubmit,
  downloadName,
  type FieldErrors,
  type Format,
  initialState,
  isPngOutput,
  type Notice,
  noticeFor,
  type Options,
  reduce,
  resizeStream,
  type State,
  type Strategy,
  StreamUnavailableError,
  sizeChange,
} from '../lib/resize'

const STRATEGIES: SelectOption<Strategy>[] = [
  { value: 'scale-to-fit', label: 'Scale to fit', description: 'Fits the whole image inside the box.' },
  {
    value: 'crop-to-fit',
    label: 'Crop to fit',
    description: "Centre-crops to the box's shape, then shrinks.",
  },
]
const FORMATS: SelectOption<Format>[] = [
  { value: 'jpeg', label: 'JPEG' },
  { value: 'png', label: 'PNG' },
  { value: 'webp', label: 'WebP' },
]
const labelOf = <T extends string>(options: SelectOption<T>[], value: T) =>
  options.find((option) => option.value === value)?.label ?? value

export function noticeCopy(notice: Notice): { headline: string; detail: string } {
  return notice.kind === 'too-large'
    ? {
        headline: `${notice.name} is too large`,
        detail: `It is ${formatBytes(notice.size)}. image_resize::resize reads images up to 64 MiB.`,
      }
    : {
        headline: 'Drop an image file',
        detail: 'Links and images dragged from a page are not supported. Save the image, then drop the file.',
      }
}

export function failureMessage(error: unknown): string {
  return error instanceof StreamUnavailableError
    ? 'The image stream did not open or was cut off. If this happens with every image, the ADE serving this page is older than the page and does not relay streams yet: update the ade worker and reload.'
    : errorMessage(error)
}

const dimensions = (width: number, height: number) => `${width} × ${height}`

type Viewer = 'source' | 'result' | null

export interface ImageResizeViewProps {
  state: State
  urls: { source: string | null; result: string | null }
  options: Options
  errors: FieldErrors
  dragging: boolean
  viewer: Viewer
  announcement: LiveAnnouncement | null
  refs?: {
    input?: Ref<HTMLInputElement>
    choose?: Ref<HTMLButtonElement>
    form?: Ref<HTMLFormElement>
    result?: Ref<HTMLDivElement>
    download?: Ref<HTMLAnchorElement>
  }
  handlers: {
    onChoose(): void
    onFile(file: File | undefined): void
    onDragging(dragging: boolean): void
    onRemove(): void
    onOption(patch: Partial<Options>): void
    onSubmit(): void
    onViewer(viewer: Viewer): void
    onPreviewLoad(width: number, height: number): void
    onPreviewError(target: 'source' | 'result'): void
  }
  onRequestClose?: () => void
}

/** The whole page as a function of its props. */
export function ImageResizeView({
  state,
  urls,
  options,
  errors,
  dragging,
  viewer,
  announcement,
  refs = {},
  handlers,
  onRequestClose,
}: ImageResizeViewProps) {
  const { source, notice, run } = state
  const png = isPngOutput(options, source?.file.type ?? '')
  const running = run.phase === 'running'
  const unavailable = (
    <StatusPanel variant="warn" headline="Preview unavailable" detail="The browser could not display this file." />
  )

  let original: MetaRowItem[] = []
  let originalDims: string | undefined
  if (source) {
    if (run.phase === 'done') originalDims = dimensions(run.result.source_width, run.result.source_height)
    else if (source.preview === 'ok' && source.width && source.height) {
      originalDims = dimensions(source.width, source.height)
    }
    original = [
      ...(originalDims ? [{ label: 'Dimensions', value: originalDims }] : []),
      { label: 'Size', value: formatBytes(source.file.size) },
      { label: 'Type', value: source.file.type || 'unknown' },
    ]
  }

  let resized: ReactNode = null
  if (source && run.phase === 'idle') {
    resized = <EmptyState compact title="Not resized yet" description="Set the options and select Resize." />
  } else if (running) {
    resized = (
      <div role="status" aria-label="Resizing">
        <Skeleton className="imgr-skeleton" />
      </div>
    )
  } else if (source && run.phase === 'failed') {
    resized = (
      <StatusPanel
        variant="alert"
        headline={`Could not resize ${source.file.name}`}
        detail={failureMessage(run.error)}
        action={
          <Button variant="secondary" onClick={handlers.onSubmit}>
            Try again
          </Button>
        }
      />
    )
  } else if (source && run.phase === 'done') {
    const { result, blob } = run
    const name = downloadName(source.file.name, result)
    const size = dimensions(result.width, result.height)
    const change = sizeChange(source.file.size, blob.size)
    resized = (
      <>
        {run.preview === 'failed' ? (
          unavailable
        ) : (
          <div className="imgr-frame">
            {urls.result && (
              <>
                <ImageThumbnailButton className="imgr-open" title={name} onClick={() => handlers.onViewer('result')}>
                  <img
                    className="imgr-image"
                    src={urls.result}
                    alt={`Resized: ${name}`}
                    onError={() => handlers.onPreviewError('result')}
                  />
                </ImageThumbnailButton>
                <ImageViewer
                  open={viewer === 'result'}
                  onOpenChange={(open) => handlers.onViewer(open ? 'result' : null)}
                  src={urls.result}
                  alt={`Resized: ${name}`}
                  title={name}
                  description={`${formatBytes(blob.size)} · ${blob.type}`}
                />
              </>
            )}
          </div>
        )}
        <MetaRow
          items={[
            { label: 'Dimensions', value: size },
            { label: 'Size', value: formatBytes(blob.size) },
            { label: 'Format', value: labelOf(FORMATS, result.format) },
            { label: 'Strategy', value: labelOf(STRATEGIES, result.strategy) },
            { label: 'Change', value: change.text, tone: change.tone },
          ]}
        />
        {urls.result && (
          <Button variant="secondary" asChild className="imgr-action" data-settings-narrow-action="">
            <a ref={refs.download} href={urls.result} download={name}>
              <Download />
              Download
            </a>
          </Button>
        )}
      </>
    )
  }

  return (
    <PageShell>
      <PageHeader
        icon={<ImageIcon />}
        title="Image resize"
        description="Downscale a JPEG, PNG or WebP image and compare it with the original."
        onClose={onRequestClose}
      />
      <PageMain className="imgr-main">
        <div className="imgr">
          <div
            className="imgr-drop"
            data-dragging={dragging || undefined}
            data-compact={source ? true : undefined}
            onDragOver={(event) => {
              event.preventDefault()
              if (!dragging) handlers.onDragging(true)
            }}
            onDragLeave={(event) => {
              // Moving over the button inside fires dragleave on the zone too.
              if (!event.currentTarget.contains(event.relatedTarget as Node | null)) handlers.onDragging(false)
            }}
            onDrop={(event) => {
              event.preventDefault()
              handlers.onDragging(false)
              handlers.onFile(event.dataTransfer.files?.[0])
            }}
          >
            <p className="imgr-drop-label">
              {dragging ? (
                source ? (
                  'Drop to replace'
                ) : (
                  'Drop to use this image'
                )
              ) : source ? (
                <>
                  <span className="imgr-drop-name">{source.file.name}</span>
                  <span className="imgr-drop-size">{formatBytes(source.file.size)}</span>
                </>
              ) : (
                'Drop a JPEG, PNG or WebP image here'
              )}
            </p>
            <Button
              ref={refs.choose}
              variant="secondary"
              size={source ? 'sm' : undefined}
              data-autofocus=""
              data-settings-narrow-action=""
              onClick={handlers.onChoose}
            >
              {source ? 'Choose another' : 'Choose an image'}
            </Button>
            {source ? (
              <IconButton label="Remove image" onClick={handlers.onRemove}>
                <Trash2 />
              </IconButton>
            ) : null}
            <input
              ref={refs.input}
              type="file"
              accept="image/jpeg,image/png,image/webp"
              hidden
              onChange={(event) => {
                const file = event.target.files?.[0]
                if (file) handlers.onFile(file)
                event.target.value = ''
              }}
            />
          </div>

          {notice && <StatusPanel variant="warn" {...noticeCopy(notice)} />}

          <form
            ref={refs.form}
            className="imgr-form"
            noValidate
            onSubmit={(event) => {
              event.preventDefault()
              handlers.onSubmit()
            }}
          >
            <SettingsSection
              title="Options"
              description="Empty options use the worker's configured defaults. The image is never enlarged."
            >
              <SettingsList>
                <SettingsField
                  field="width"
                  label="Width"
                  controlSize="compact"
                  error={errors.width}
                  renderControl={(props) => (
                    <Input
                      {...props}
                      // A number input hands "" for text it cannot parse ("200px"), which would read as the default.
                      type="text"
                      inputMode="numeric"
                      placeholder="Default"
                      value={options.width}
                      onChange={(width) => handlers.onOption({ width })}
                    />
                  )}
                />
                <SettingsField
                  field="height"
                  label="Height"
                  controlSize="compact"
                  error={errors.height}
                  renderControl={(props) => (
                    <Input
                      {...props}
                      type="text"
                      inputMode="numeric"
                      placeholder="Default"
                      value={options.height}
                      onChange={(height) => handlers.onOption({ height })}
                    />
                  )}
                />
                <SettingsField
                  field="strategy"
                  label="Strategy"
                  renderControl={(props) => (
                    <Select
                      {...props}
                      value={options.strategy}
                      options={STRATEGIES}
                      onChange={(strategy) => handlers.onOption({ strategy })}
                      allowEmpty
                      emptyLabel="Default"
                      placeholder="Default"
                      onClear={() => handlers.onOption({ strategy: undefined })}
                      sheetTitle="Strategy"
                    />
                  )}
                />
                <SettingsField
                  field="format"
                  label="Format"
                  renderControl={(props) => (
                    <Select
                      {...props}
                      value={options.format}
                      options={FORMATS}
                      onChange={(format) => handlers.onOption({ format })}
                      allowEmpty
                      emptyLabel="Same as the input"
                      placeholder="Same as the input"
                      onClear={() => handlers.onOption({ format: undefined })}
                      sheetTitle="Format"
                    />
                  )}
                />
                <SettingsField
                  field="quality"
                  label="Quality"
                  description={png ? 'PNG is lossless; quality does not apply.' : '1 to 100, for JPEG and WebP.'}
                  controlSize="compact"
                  error={png ? undefined : errors.quality}
                  renderControl={(props) => (
                    <Input
                      {...props}
                      type="text"
                      inputMode="numeric"
                      placeholder="Default"
                      disabled={png}
                      value={options.quality}
                      onChange={(quality) => handlers.onOption({ quality })}
                    />
                  )}
                />
              </SettingsList>
            </SettingsSection>
            <Button
              variant="primary"
              type="submit"
              className="imgr-action"
              data-settings-narrow-action=""
              disabled={!canSubmit(state)}
            >
              {running ? 'Resizing…' : 'Resize'}
            </Button>
          </form>

          {source ? (
            <div className="imgr-figures">
              <div className="imgr-figure">
                <Eyebrow as="h2">Original</Eyebrow>
                {source.preview === 'failed' ? (
                  unavailable
                ) : (
                  <div className="imgr-frame">
                    {urls.source && (
                      <>
                        <ImageThumbnailButton
                          className="imgr-open"
                          title={source.file.name}
                          onClick={() => handlers.onViewer('source')}
                        >
                          <img
                            className="imgr-image"
                            src={urls.source}
                            alt={`Original: ${source.file.name}`}
                            onLoad={(event) =>
                              handlers.onPreviewLoad(
                                event.currentTarget.naturalWidth,
                                event.currentTarget.naturalHeight,
                              )
                            }
                            onError={() => handlers.onPreviewError('source')}
                          />
                        </ImageThumbnailButton>
                        <ImageViewer
                          open={viewer === 'source'}
                          onOpenChange={(open) => handlers.onViewer(open ? 'source' : null)}
                          src={urls.source}
                          alt={`Original: ${source.file.name}`}
                          title={source.file.name}
                          description={`${formatBytes(source.file.size)} · ${source.file.type || 'unknown'}`}
                        />
                      </>
                    )}
                  </div>
                )}
                <MetaRow items={original} />
              </div>
              <div className="imgr-figure" ref={refs.result} tabIndex={-1}>
                <Eyebrow as="h2">Resized</Eyebrow>
                {resized}
              </div>
            </div>
          ) : (
            <EmptyState
              title="No image yet"
              description="Images up to 64 MiB can be resized here and compared with the original."
            />
          )}

          <LiveRegion announcement={announcement} />
        </div>
      </PageMain>
    </PageShell>
  )
}

/** Whether a call would drop `active` to the document: Resize disables, the Resized figure's controls unmount. */
export function focusFallsWithCall(active: Element | null, figure: Element | null): boolean {
  return !!active && (!!figure?.contains(active) || (active instanceof HTMLButtonElement && active.type === 'submit'))
}

/** An object URL for `blob`, revoked when the blob changes or the page unmounts. */
function useObjectUrl(blob: Blob | null): string | null {
  const [entry, setEntry] = useState<{ blob: Blob; url: string } | null>(null)
  useEffect(() => {
    if (!blob) return
    const url = URL.createObjectURL(blob)
    setEntry({ blob, url })
    return () => URL.revokeObjectURL(url)
  }, [blob])
  return entry?.blob === blob ? entry.url : null
}

export function ImageResizePage({ host, onRequestClose, commands }: { host: Host } & Partial<PageRenderProps>) {
  const [state, dispatch] = useReducer(reduce, initialState)
  const [options, setOptions] = useState<Options>({ width: '', height: '', quality: '' })
  const [errors, setErrors] = useState<FieldErrors>({})
  const [dragging, setDragging] = useState(false)
  const [viewer, setViewer] = useState<Viewer>(null)
  const [announcement, setAnnouncement] = useState<LiveAnnouncement | null>(null)
  // The generation of the latest pick, remove or submit, and of the call in flight: Mod+Enter and the
  // form's implicit submit can both fire for one key press, before React state says the call runs.
  const generation = useRef(0)
  const inFlight = useRef<number | null>(null)
  const input = useRef<HTMLInputElement>(null)
  const choose = useRef<HTMLButtonElement>(null)
  const form = useRef<HTMLFormElement>(null)
  const result = useRef<HTMLDivElement>(null)
  const download = useRef<HTMLAnchorElement>(null)
  const sourceUrl = useObjectUrl(state.source?.file ?? null)
  const resultUrl = useObjectUrl(state.run.phase === 'done' ? state.run.blob : null)

  const announce = (text: string, urgency: LiveAnnouncement['urgency'] = 'polite') =>
    setAnnouncement((previous) => ({ seq: (previous?.seq ?? 0) + 1, text, urgency }))

  const pick = (file: File | undefined) => {
    if (file) generation.current += 1
    dispatch({ type: 'pick', generation: generation.current, file })
    const notice = noticeFor(file)
    if (notice) announce(noticeCopy(notice).headline, 'assertive')
  }

  const remove = () => {
    generation.current += 1
    dispatch({ type: 'remove', generation: generation.current })
    choose.current?.focus()
  }

  const submit = () => {
    const source = state.source
    if (!source || inFlight.current === generation.current) return
    const built = buildMetadata(options, source.file.type)
    if ('errors' in built) {
      setErrors(built.errors)
      const fields = Object.keys(built.errors)
      announce(`Fix ${fields.length} ${fields.length === 1 ? 'option' : 'options'} to resize`, 'assertive')
      form.current?.querySelector<HTMLElement>(`[data-field="${fields[0]}"]`)?.focus()
      return
    }
    setErrors({})
    const call = ++generation.current
    inFlight.current = call
    if (focusFallsWithCall(document.activeElement, result.current)) result.current?.focus({ preventScroll: true })
    dispatch({ type: 'submit', generation: call })
    announce(`Resizing ${source.file.name}`)
    result.current?.scrollIntoView({ block: 'nearest' })
    resizeStream(host.iii, source.file, built.metadata)
      .then(
        (done) => {
          dispatch({ type: 'settled', generation: call, ...done })
          if (call === generation.current) {
            announce(`Resized to ${dimensions(done.result.width, done.result.height)}, ${formatBytes(done.blob.size)}`)
          }
        },
        (error: unknown) => {
          dispatch({ type: 'settled', generation: call, error })
          if (call === generation.current) {
            announce(`Could not resize ${source.file.name}: ${failureMessage(error)}`, 'assertive')
          }
        },
      )
      .finally(() => {
        if (inFlight.current === call) inFlight.current = null
      })
  }

  const latest = useRef({ state, submit })
  useEffect(() => {
    latest.current = { state, submit }
  })

  useEffect(
    () =>
      commands?.register([
        {
          id: 'choose',
          title: 'Choose an image',
          detail: 'Pick a JPEG, PNG or WebP image to resize',
          keywords: ['open', 'upload', 'drop'],
          run: () => input.current?.click(),
        },
        {
          id: 'resize',
          title: 'Resize',
          detail: 'Resize the image with the current options',
          shortcut: 'Mod+Enter',
          firesWhileTyping: true,
          enabled: () => canSubmit(latest.current.state),
          run: () => latest.current.submit(),
        },
        {
          id: 'download',
          title: 'Download the resized image',
          enabled: () => latest.current.state.run.phase === 'done',
          run: () => download.current?.click(),
        },
      ]),
    [commands],
  )

  return (
    <ImageResizeView
      state={state}
      urls={{ source: sourceUrl, result: resultUrl }}
      options={options}
      errors={errors}
      dragging={dragging}
      viewer={viewer}
      announcement={announcement}
      refs={{ input, choose, form, result, download }}
      handlers={{
        onChoose: () => input.current?.click(),
        onFile: pick,
        onDragging: setDragging,
        onRemove: remove,
        onOption: (patch) => {
          setOptions((previous) => ({ ...previous, ...patch }))
          setErrors((previous) => {
            const next = { ...previous }
            for (const key of Object.keys(patch)) delete next[key as keyof FieldErrors]
            return next
          })
        },
        onSubmit: submit,
        onViewer: setViewer,
        onPreviewLoad: (width, height) => dispatch({ type: 'previewLoaded', width, height }),
        onPreviewError: (target) => dispatch({ type: 'previewFailed', target }),
      }}
      onRequestClose={onRequestClose}
    />
  )
}
