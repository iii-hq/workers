import {
  Button,
  EmptyState,
  type Host,
  type PageCommandsApi,
  PageHeader,
  type PageRenderProps,
  PageShell,
  PageSidebar,
  type PanelContextEvent,
  StatusPanel,
} from '@iii-dev/console-ui'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { parseStoragePanelContext } from './panel-context'
import {
  BackIcon,
  BucketIcon,
  ChevronIcon,
  DownloadIcon,
  FileIcon,
  FolderIcon,
  formatBytes,
  leafName,
  parentPrefix,
  RefreshIcon,
  StorageIcon,
  TrashIcon,
  UploadIcon,
  useContainerNarrow,
} from './widgets'

const NARROW_BELOW = 760
const LIST_LIMIT = 250

interface BucketSummary {
  name: string
  provider: string
}

interface StorageObject {
  key: string
  etag: string
  size: number
  last_modified: string
  content_type?: string | null
}

interface ObjectMetadata {
  content_type: string
  etag: string
  last_modified: string
  size: number
}

interface Listing {
  objects: StorageObject[]
  common_prefixes: string[]
  next_cursor?: string | null
}

interface PersistedState {
  bucket?: string
  prefix?: string
  objectKey?: string
}

function readPersisted(tabId: string): PersistedState {
  if (!tabId) return {}
  try {
    return JSON.parse(
      localStorage.getItem(`iii:storage-ui:${tabId}`) ?? '{}',
    ) as PersistedState
  } catch {
    return {}
  }
}

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error)
}

function StorageExplorer({
  host,
  panelSide,
  tabId,
  panelContext,
  commands,
}: {
  host: Host
  panelSide: 'left' | 'right'
  tabId: string
  panelContext?: PanelContextEvent
  commands?: PageCommandsApi
}) {
  const persisted = useMemo(() => readPersisted(tabId), [tabId])
  const [rootRef, narrow] = useContainerNarrow(NARROW_BELOW)
  const [buckets, setBuckets] = useState<BucketSummary[] | null>(null)
  const [bucketError, setBucketError] = useState<string | null>(null)
  const [bucketName, setBucketName] = useState<string | null>(
    persisted.bucket ?? null,
  )
  const [prefix, setPrefix] = useState(persisted.prefix ?? '')
  const [listing, setListing] = useState<Listing | null>(null)
  const [listingError, setListingError] = useState<string | null>(null)
  const [objectKey, setObjectKey] = useState<string | null>(
    persisted.objectKey ?? null,
  )
  const [metadata, setMetadata] = useState<ObjectMetadata | null>(null)
  const [metadataError, setMetadataError] = useState<string | null>(null)
  const [loadingMore, setLoadingMore] = useState(false)
  const [transfer, setTransfer] = useState<string | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)
  const fileInputRef = useRef<HTMLInputElement | null>(null)
  const bucketRef = useRef(bucketName)
  const prefixRef = useRef(prefix)
  const objectRef = useRef(objectKey)
  bucketRef.current = bucketName
  prefixRef.current = prefix
  objectRef.current = objectKey

  const selectedBucket =
    buckets?.find((bucket) => bucket.name === bucketName) ?? null

  useEffect(() => {
    if (!tabId) return
    try {
      localStorage.setItem(
        `iii:storage-ui:${tabId}`,
        JSON.stringify({ bucket: bucketName, prefix, objectKey }),
      )
    } catch {
      // Per-tab persistence is best-effort.
    }
  }, [tabId, bucketName, prefix, objectKey])

  const loadBuckets = useCallback(() => {
    setBucketError(null)
    host.iii
      .trigger<{ buckets: BucketSummary[] }>('storage::listBuckets', {})
      .then((response) => {
        setBuckets(response.buckets)
        setBucketError(null)
        if (
          bucketRef.current &&
          !response.buckets.some((bucket) => bucket.name === bucketRef.current)
        ) {
          setBucketName(null)
          setPrefix('')
          setObjectKey(null)
        }
      })
      .catch((error: unknown) => {
        setBuckets([])
        setBucketError(errorMessage(error))
      })
  }, [host])

  useEffect(loadBuckets, [loadBuckets])

  const loadListing = useCallback(
    (cursor?: string, append = false) => {
      const currentBucket = bucketRef.current
      const currentPrefix = prefixRef.current
      if (!currentBucket) return
      if (append) setLoadingMore(true)
      else setListing(null)
      setListingError(null)
      host.iii
        .trigger<Listing>('storage::listObjects', {
          bucket: currentBucket,
          prefix: currentPrefix,
          delimiter: '/',
          limit: LIST_LIMIT,
          ...(cursor ? { cursor } : {}),
        })
        .then((response) => {
          if (
            bucketRef.current !== currentBucket ||
            prefixRef.current !== currentPrefix
          )
            return
          setListing((previous) =>
            append && previous
              ? {
                  objects: [...previous.objects, ...response.objects],
                  common_prefixes: [
                    ...previous.common_prefixes,
                    ...response.common_prefixes,
                  ],
                  next_cursor: response.next_cursor,
                }
              : response,
          )
        })
        .catch((error: unknown) => {
          if (
            bucketRef.current !== currentBucket ||
            prefixRef.current !== currentPrefix
          )
            return
          setListingError(errorMessage(error))
          if (!append) setListing({ objects: [], common_prefixes: [] })
        })
        .finally(() => setLoadingMore(false))
    },
    [host],
  )

  useEffect(() => {
    if (bucketName) loadListing()
    else setListing(null)
  }, [bucketName, prefix, loadListing])

  useEffect(() => {
    if (!bucketName || !objectKey) {
      setMetadata(null)
      setMetadataError(null)
      return
    }
    const currentBucket = bucketName
    const currentObject = objectKey
    setMetadata(null)
    setMetadataError(null)
    host.iii
      .trigger<ObjectMetadata>('storage::headObject', {
        bucket: currentBucket,
        key: currentObject,
      })
      .then((response) => {
        if (
          bucketRef.current === currentBucket &&
          objectRef.current === currentObject
        ) {
          setMetadata(response)
        }
      })
      .catch((error: unknown) => {
        if (
          bucketRef.current === currentBucket &&
          objectRef.current === currentObject
        ) {
          setMetadataError(errorMessage(error))
        }
      })
  }, [host, bucketName, objectKey])

  // A context from the palette source (or another worker's "inspect this
  // object" affordance) selects a bucket/prefix/object once the bucket list
  // has loaded — the page can mount before the first fetch resolves.
  const appliedContextRef = useRef(0)
  useEffect(() => {
    if (!panelContext || panelContext.id === appliedContextRef.current) return
    const context = parseStoragePanelContext(panelContext.context)
    if (!context) {
      appliedContextRef.current = panelContext.id
      return
    }
    if (buckets === null || bucketError) return
    if (!buckets.some((b) => b.name === context.bucket)) {
      appliedContextRef.current = panelContext.id
      return
    }
    appliedContextRef.current = panelContext.id
    setBucketName(context.bucket)
    setPrefix(context.prefix ?? '')
    setObjectKey(context.objectKey ?? null)
    setActionError(null)
  }, [panelContext, buckets, bucketError])

  const openBucket = (name: string) => {
    setBucketName(name)
    setPrefix('')
    setObjectKey(null)
    setActionError(null)
  }

  const openFolder = (nextPrefix: string) => {
    setPrefix(nextPrefix)
    setObjectKey(null)
    setActionError(null)
  }

  const goBack = () => {
    if (objectKey) {
      setObjectKey(null)
    } else if (prefix) {
      openFolder(parentPrefix(prefix))
    } else {
      setBucketName(null)
    }
  }

  const upload = async (file: File) => {
    if (!bucketName || !selectedBucket) return
    const key = `${prefix}${file.name}`
    if (
      listing?.objects.some((object) => object.key === key) &&
      !window.confirm(`Replace ${file.name}?`)
    ) {
      return
    }
    const contentType = file.type || 'application/octet-stream'
    setTransfer(`uploading ${file.name}`)
    setActionError(null)
    try {
      if (selectedBucket.provider === 'local') {
        const signed = await host.iii.trigger<{
          url: string
          fields: Record<string, string>
        }>('storage::presignPost', {
          bucket: bucketName,
          key,
          content_type: contentType,
          expires_in_seconds: 600,
        })
        const body = new FormData()
        for (const [field, value] of Object.entries(signed.fields))
          body.append(field, value)
        body.append('file', file)
        const response = await fetch(signed.url, { method: 'POST', body })
        if (!response.ok)
          throw new Error(`upload failed with HTTP ${response.status}`)
      } else {
        const signed = await host.iii.trigger<{ url: string }>(
          'storage::presignUrl',
          {
            bucket: bucketName,
            key,
            method: 'PUT',
            content_type: contentType,
            expires_in_seconds: 600,
          },
        )
        const response = await fetch(signed.url, {
          method: 'PUT',
          headers: { 'Content-Type': contentType },
          body: file,
        })
        if (!response.ok)
          throw new Error(`upload failed with HTTP ${response.status}`)
      }
      loadListing()
      setObjectKey(key)
    } catch (error) {
      setActionError(errorMessage(error))
    } finally {
      setTransfer(null)
      if (fileInputRef.current) fileInputRef.current.value = ''
    }
  }

  const download = async () => {
    if (!bucketName || !objectKey) return
    const popup = window.open('', '_blank')
    setTransfer(`preparing ${leafName(objectKey)}`)
    setActionError(null)
    try {
      const signed = await host.iii.trigger<{ url: string }>(
        'storage::presignUrl',
        {
          bucket: bucketName,
          key: objectKey,
          method: 'GET',
          expires_in_seconds: 600,
          response_content_disposition: `attachment; filename="${leafName(objectKey).replaceAll('"', '')}"`,
        },
      )
      if (popup) popup.location.href = signed.url
      else window.location.assign(signed.url)
    } catch (error) {
      popup?.close()
      setActionError(errorMessage(error))
    } finally {
      setTransfer(null)
    }
  }

  const remove = async () => {
    if (
      !bucketName ||
      !objectKey ||
      !window.confirm(`Delete ${leafName(objectKey)}?`)
    )
      return
    const key = objectKey
    setTransfer(`deleting ${leafName(key)}`)
    setActionError(null)
    try {
      await host.iii.trigger('storage::deleteObject', {
        bucket: bucketName,
        key,
      })
      setObjectKey(null)
      loadListing()
    } catch (error) {
      setActionError(errorMessage(error))
    } finally {
      setTransfer(null)
    }
  }

  // The page's verbs, for the palette and for the keyboard while this pane
  // has the focus. The keys stay clear of the console's own.
  useEffect(
    () =>
      commands?.register([
        {
          id: 'upload',
          title: 'Upload a file',
          detail: 'Choose a file to upload into the current folder',
          keywords: ['file', 'add'],
          enabled: () => bucketName !== null && transfer === null,
          run: () => fileInputRef.current?.click(),
        },
        {
          id: 'refresh',
          title: 'Refresh',
          detail: 'Reload buckets and the current folder',
          keywords: ['reload'],
          run: () => {
            loadBuckets()
            if (bucketName) loadListing()
          },
        },
      ]),
    [commands, bucketName, transfer, loadBuckets, loadListing],
  )

  const showBuckets = !narrow || bucketName === null
  const showContents = !narrow || (bucketName !== null && objectKey === null)
  const showDetail = !narrow || objectKey !== null
  const rowCount =
    (listing?.common_prefixes.length ?? 0) + (listing?.objects.length ?? 0)

  return (
    <div
      ref={rootRef}
      className={`storage-ui-browser${narrow ? ' narrow' : ''}${panelSide === 'right' ? ' right' : ''}`}
    >
      {showBuckets ? (
        <PageSidebar
          label="buckets"
          side={panelSide}
          collapsible
          storageKey="storage:buckets"
          defaultWidth={204}
          narrow={narrow}
          className="storage-ui-buckets"
          data-autofocus=""
          tabIndex={-1}
          header={
            <div className="storage-ui-column-head storage-ui-primary-head">
              <span className="storage-ui-column-label">buckets</span>
              <span className="storage-ui-spacer" />
              {buckets ? (
                <span className="storage-ui-count">{buckets.length}</span>
              ) : null}
              <button
                type="button"
                className="storage-ui-icon-button"
                onClick={loadBuckets}
                aria-label="Refresh buckets"
              >
                <RefreshIcon />
              </button>
            </div>
          }
        >
          <div className="storage-ui-scroll">
            {buckets === null ? (
              <div className="storage-ui-skeleton" aria-label="Loading buckets">
                <span />
                <span />
                <span />
              </div>
            ) : bucketError ? (
              <div className="storage-ui-inline-error">
                <p>Could not load buckets.</p>
                <Button variant="ghost" size="sm" onClick={loadBuckets}>
                  retry
                </Button>
              </div>
            ) : buckets.length === 0 ? (
              <div className="storage-ui-column-empty">
                <p>No buckets configured.</p>
                <p>Add one in the storage worker configuration.</p>
              </div>
            ) : (
              <ul className="storage-ui-nav-list" role="list">
                {buckets.map((bucket) => (
                  <li key={bucket.name}>
                    <button
                      type="button"
                      className={`storage-ui-bucket-row${bucket.name === bucketName ? ' active' : ''}`}
                      aria-current={
                        bucket.name === bucketName ? 'true' : undefined
                      }
                      onClick={() => openBucket(bucket.name)}
                    >
                      <BucketIcon className="storage-ui-row-icon" />
                      <span className="storage-ui-row-copy">
                        <span className="storage-ui-row-name">
                          {bucket.name}
                        </span>
                        <span className="storage-ui-row-meta">
                          {bucket.provider}
                        </span>
                      </span>
                      <ChevronIcon className="storage-ui-chevron" />
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </div>
        </PageSidebar>
      ) : null}

      {showContents ? (
        <section className="storage-ui-contents" aria-label="Bucket contents">
          {bucketName === null ? (
            <>
              <header className="storage-ui-column-head">
                <span className="storage-ui-column-label">objects</span>
              </header>
              <div className="storage-ui-column-empty roomy">
                <p>Select a bucket to browse its objects.</p>
              </div>
            </>
          ) : (
            <>
              <header className="storage-ui-path-head">
                {narrow || prefix ? (
                  <button
                    type="button"
                    className="storage-ui-back-button"
                    onClick={goBack}
                    aria-label={
                      prefix ? 'Go to parent folder' : 'Back to buckets'
                    }
                  >
                    <BackIcon />
                  </button>
                ) : null}
                <div className="storage-ui-path-copy">
                  <span className="storage-ui-path-bucket">{bucketName}</span>
                  <span
                    className="storage-ui-path-prefix"
                    title={prefix || '/'}
                  >
                    {prefix || '/'}
                  </span>
                </div>
                <span className="storage-ui-spacer" />
                {listing ? (
                  <span className="storage-ui-count">{rowCount}</span>
                ) : null}
                <button
                  type="button"
                  className="storage-ui-icon-button"
                  onClick={() => loadListing()}
                  aria-label="Refresh folder"
                >
                  <RefreshIcon />
                </button>
              </header>
              <div className="storage-ui-work-toolbar">
                <div
                  className="storage-ui-breadcrumb"
                  aria-label="Current folder"
                >
                  <button type="button" onClick={() => openFolder('')}>
                    {bucketName}
                  </button>
                  {prefix
                    .split('/')
                    .filter(Boolean)
                    .map((segment, index, parts) => {
                      const value = `${parts.slice(0, index + 1).join('/')}/`
                      return (
                        <span key={value}>
                          <span aria-hidden="true">/</span>
                          <button
                            type="button"
                            onClick={() => openFolder(value)}
                          >
                            {segment}
                          </button>
                        </span>
                      )
                    })}
                </div>
                <input
                  ref={fileInputRef}
                  className="storage-ui-file-input"
                  type="file"
                  name="storage-upload"
                  aria-label="Choose a file to upload"
                  onChange={(event) => {
                    const file = event.currentTarget.files?.[0]
                    if (file) void upload(file)
                  }}
                />
                <Button
                  variant="primary"
                  size="sm"
                  disabled={transfer !== null}
                  onClick={() => fileInputRef.current?.click()}
                >
                  <UploadIcon className="storage-ui-button-icon" /> upload
                </Button>
              </div>
              {actionError && !objectKey ? (
                <StatusPanel
                  variant="alert"
                  headline="Transfer failed"
                  detail={actionError}
                  className="storage-ui-status"
                />
              ) : null}
              <div className="storage-ui-scroll">
                {listing === null ? (
                  <div
                    className="storage-ui-skeleton listing"
                    aria-label="Loading objects"
                  >
                    <span />
                    <span />
                    <span />
                    <span />
                  </div>
                ) : listingError ? (
                  <div className="storage-ui-inline-error">
                    <p>Could not load this folder.</p>
                    <span>{listingError}</span>
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => loadListing()}
                    >
                      retry
                    </Button>
                  </div>
                ) : rowCount === 0 ? (
                  <EmptyState
                    icon={FolderIcon}
                    title="This folder is empty"
                    description="Upload a file here, or choose another bucket or folder."
                    action={{
                      label: 'upload file',
                      onClick: () => fileInputRef.current?.click(),
                    }}
                  />
                ) : (
                  <ul className="storage-ui-object-list" role="list">
                    {listing.common_prefixes.map((folder) => (
                      <li key={`folder:${folder}`}>
                        <button
                          type="button"
                          className="storage-ui-object-row"
                          onClick={() => openFolder(folder)}
                        >
                          <FolderIcon className="storage-ui-row-icon folder" />
                          <span className="storage-ui-row-copy">
                            <span className="storage-ui-row-name">
                              {leafName(folder)}
                            </span>
                            <span className="storage-ui-row-meta">folder</span>
                          </span>
                          <ChevronIcon className="storage-ui-chevron" />
                        </button>
                      </li>
                    ))}
                    {listing.objects.map((object) => (
                      <li key={`object:${object.key}`}>
                        <button
                          type="button"
                          className={`storage-ui-object-row${object.key === objectKey ? ' active' : ''}`}
                          aria-current={
                            object.key === objectKey ? 'true' : undefined
                          }
                          onClick={() => setObjectKey(object.key)}
                        >
                          <FileIcon className="storage-ui-row-icon" />
                          <span className="storage-ui-row-copy">
                            <span className="storage-ui-row-name">
                              {leafName(object.key)}
                            </span>
                            <span className="storage-ui-row-meta">
                              {formatBytes(object.size)} ·{' '}
                              {new Date(
                                object.last_modified,
                              ).toLocaleDateString()}
                            </span>
                          </span>
                          <ChevronIcon className="storage-ui-chevron" />
                        </button>
                      </li>
                    ))}
                  </ul>
                )}
                {listing?.next_cursor ? (
                  <div className="storage-ui-load-more">
                    <Button
                      variant="ghost"
                      size="sm"
                      disabled={loadingMore}
                      onClick={() =>
                        loadListing(listing.next_cursor ?? undefined, true)
                      }
                    >
                      {loadingMore ? 'loading…' : 'load more'}
                    </Button>
                  </div>
                ) : null}
              </div>
              {transfer && !objectKey ? (
                <div className="storage-ui-transfer" role="status">
                  {transfer}…
                </div>
              ) : null}
            </>
          )}
        </section>
      ) : null}

      {showDetail ? (
        <main className="storage-ui-detail" aria-label="Object details">
          {bucketName === null || objectKey === null ? (
            <div className="storage-ui-hero">
              <StorageIcon className="storage-ui-hero-icon" />
              <h2>Select an object</h2>
              <p>
                Browse a bucket and its folders, then select a file to inspect
                its metadata or create a signed download.
              </p>
              <div
                className="storage-ui-hero-flow"
                aria-label="Navigation levels"
              >
                <span>bucket</span>
                <ChevronIcon />
                <span>folder</span>
                <ChevronIcon />
                <span>object</span>
              </div>
            </div>
          ) : (
            <div className="storage-ui-inspector">
              <header className="storage-ui-inspector-head">
                {narrow ? (
                  <button
                    type="button"
                    className="storage-ui-back-button"
                    onClick={goBack}
                    aria-label="Back to folder"
                  >
                    <BackIcon />
                  </button>
                ) : null}
                <FileIcon className="storage-ui-inspector-icon" />
                <div className="storage-ui-inspector-title">
                  <h2>{leafName(objectKey)}</h2>
                  <p title={objectKey}>{objectKey}</p>
                </div>
              </header>
              <div className="storage-ui-inspector-actions">
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={transfer !== null}
                  onClick={() => void download()}
                >
                  <DownloadIcon className="storage-ui-button-icon" /> download
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={transfer !== null}
                  onClick={() => void navigator.clipboard.writeText(objectKey)}
                >
                  copy key
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={transfer !== null}
                  onClick={() => void remove()}
                >
                  <TrashIcon className="storage-ui-button-icon" /> delete
                </Button>
              </div>
              {actionError ? (
                <StatusPanel
                  variant="alert"
                  headline="Action failed"
                  detail={actionError}
                  className="storage-ui-status"
                />
              ) : null}
              {metadataError ? (
                <StatusPanel
                  variant="alert"
                  headline="Metadata unavailable"
                  detail={metadataError}
                  className="storage-ui-status"
                />
              ) : metadata === null ? (
                <div
                  className="storage-ui-detail-skeleton"
                  aria-label="Loading metadata"
                >
                  <span />
                  <span />
                  <span />
                  <span />
                </div>
              ) : (
                <dl className="storage-ui-facts">
                  <div>
                    <dt>Size</dt>
                    <dd>{formatBytes(metadata.size)}</dd>
                  </div>
                  <div>
                    <dt>Content type</dt>
                    <dd>{metadata.content_type}</dd>
                  </div>
                  <div>
                    <dt>Last modified</dt>
                    <dd>{new Date(metadata.last_modified).toLocaleString()}</dd>
                  </div>
                  <div>
                    <dt>ETag</dt>
                    <dd title={metadata.etag}>{metadata.etag}</dd>
                  </div>
                  <div>
                    <dt>Provider</dt>
                    <dd>{selectedBucket?.provider ?? 'unknown'}</dd>
                  </div>
                </dl>
              )}
              <div className="storage-ui-transfer-note">
                <h3>Direct transfer</h3>
                <p>
                  Downloads use a short-lived signed URL, so file bytes bypass
                  the inline worker RPC limit.
                </p>
              </div>
              {transfer ? (
                <div className="storage-ui-transfer" role="status">
                  {transfer}…
                </div>
              ) : null}
            </div>
          )}
        </main>
      ) : null}
    </div>
  )
}

export function StoragePage({
  host,
  panelSide = 'left',
  tabId = '',
  onRequestClose,
  panelContext,
  commands,
}: { host: Host } & Partial<PageRenderProps>) {
  return (
    <PageShell className="storage-ui-shell">
      <PageHeader
        icon={<StorageIcon />}
        title="Storage"
        description="Buckets, folders, and objects"
        onClose={onRequestClose}
      />
      <StorageExplorer
        host={host}
        panelSide={panelSide}
        tabId={tabId}
        panelContext={panelContext}
        commands={commands}
      />
    </PageShell>
  )
}
