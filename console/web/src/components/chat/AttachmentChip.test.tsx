/**
 * Render assertions go through `renderToStaticMarkup` (no jsdom —
 * console/web's convention); the observer wiring and the download path are
 * plain functions and are driven directly.
 */

import { renderToStaticMarkup } from 'react-dom/server'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { observeFirstIntersection } from '@/hooks/use-lazy-attachment'
import {
  clearAttachmentCache,
  loadAttachment,
} from '@/lib/attachments/attachment-cache'
import type { Attachment } from '@/types/chat'
import {
  AttachmentChip,
  downloadStoredAttachment,
  isLazyImage,
} from './AttachmentChip'

const lazyShot: Attachment = {
  id: 'a_1',
  name: 'shot.png',
  size: 4096,
  type: 'image/png',
  attachmentId: 'a_1',
}

function stored(data = 'AAAA') {
  return {
    attachment: {
      attachment_id: 'a_1',
      session_id: 's-1',
      name: 'original.png',
      mime: 'image/png',
      size: 3,
      sha256: 'deadbeef',
      created_at: 1,
    },
    data,
  }
}

/** The subset of IntersectionObserver the hook touches, scripted by hand. */
class FakeObserver {
  static instances: FakeObserver[] = []
  observed: Element[] = []
  disconnected = false
  constructor(private readonly callback: IntersectionObserverCallback) {
    FakeObserver.instances.push(this)
  }
  observe(element: Element) {
    this.observed.push(element)
  }
  disconnect() {
    this.disconnected = true
  }
  fire(isIntersecting: boolean) {
    this.callback(
      [{ isIntersecting } as IntersectionObserverEntry],
      this as unknown as IntersectionObserver,
    )
  }
}

describe('isLazyImage', () => {
  it('is a picture with a stored id and no bytes here, in a session', () => {
    expect(isLazyImage(lazyShot, 's-1')).toBe(true)
  })

  /* Composer chips never pass a session: a draft chip restored with an id is
     hydrated into a File by ChatView, not fetched by the chip. */
  it('is never a composer chip', () => {
    expect(isLazyImage(lazyShot, undefined)).toBe(false)
  })

  it('is not a chip that already has its bytes', () => {
    expect(
      isLazyImage(
        { ...lazyShot, dataUrl: 'data:image/png;base64,AAAA' },
        's-1',
      ),
    ).toBe(false)
    expect(
      isLazyImage(
        {
          ...lazyShot,
          file: new File(['x'], 'shot.png', { type: 'image/png' }),
        },
        's-1',
      ),
    ).toBe(false)
  })

  it('is not a document', () => {
    expect(isLazyImage({ ...lazyShot, type: 'application/pdf' }, 's-1')).toBe(
      false,
    )
  })
})

describe('AttachmentChip · lazy image', () => {
  afterEach(() => clearAttachmentCache())

  it('renders a placeholder with the name, size and image icon before the bytes arrive', () => {
    const html = renderToStaticMarkup(
      <AttachmentChip attachment={lazyShot} sessionId="s-1" />,
    )
    expect(html).toContain('data-lazy-image="pending"')
    expect(html).toContain('shot.png')
    expect(html).toContain('4kb')
    expect(html).toContain('lucide-image')
    expect(html).not.toContain('<img')
    // The download arrow is there: the original is in the store either way.
    expect(html).toContain('aria-label="download shot.png"')
  })

  /* A chip mounting for the second time must draw the picture on its first
     paint, from the cache, with no placeholder and no request. */
  it('shows the cached thumbnail immediately when the bytes are already here', async () => {
    const trigger = vi.fn(async () => stored('BBBB'))
    await loadAttachment('s-1', 'a_1', trigger)
    const html = renderToStaticMarkup(
      <AttachmentChip attachment={lazyShot} sessionId="s-1" />,
    )
    expect(html).toContain('data-lazy-image="ready"')
    expect(html).toContain('<img src="data:image/png;base64,BBBB"')
    expect(trigger).toHaveBeenCalledTimes(1)
  })

  it('renders an inline chip exactly as before, without the lazy marker', () => {
    const html = renderToStaticMarkup(
      <AttachmentChip
        attachment={{ ...lazyShot, dataUrl: 'data:image/png;base64,AAAA' }}
        sessionId="s-1"
      />,
    )
    expect(html).not.toContain('data-lazy-image')
    expect(html).toContain('<img src="data:image/png;base64,AAAA"')
  })
})

describe('observeFirstIntersection', () => {
  const element = {} as Element

  afterEach(() => {
    FakeObserver.instances = []
    Reflect.deleteProperty(globalThis, 'IntersectionObserver')
  })

  /* The bytes are fetched once, when the chip first comes on screen, and
     not again for later intersections or a second callback. */
  it('fires once, on the first intersection, then stops watching', () => {
    vi.stubGlobal('IntersectionObserver', FakeObserver)
    const onVisible = vi.fn()
    observeFirstIntersection(element, onVisible)
    const [observer] = FakeObserver.instances
    expect(observer.observed).toEqual([element])
    observer.fire(false)
    expect(onVisible).not.toHaveBeenCalled()
    observer.fire(true)
    expect(onVisible).toHaveBeenCalledTimes(1)
    expect(observer.disconnected).toBe(true)
    observer.fire(true)
    expect(onVisible).toHaveBeenCalledTimes(1)
    vi.unstubAllGlobals()
  })

  it('stops watching when the cleanup runs before anything intersected', () => {
    vi.stubGlobal('IntersectionObserver', FakeObserver)
    const onVisible = vi.fn()
    const stop = observeFirstIntersection(element, onVisible)
    stop()
    expect(FakeObserver.instances[0].disconnected).toBe(true)
    expect(onVisible).not.toHaveBeenCalled()
    vi.unstubAllGlobals()
  })

  /* No observer (a test runtime, an old embedder) means fetch now: a chip
     that never loads is worse than one that loads early. */
  it('loads immediately where IntersectionObserver does not exist', () => {
    expect(typeof globalThis.IntersectionObserver).toBe('undefined')
    const onVisible = vi.fn()
    observeFirstIntersection(element, onVisible)
    expect(onVisible).toHaveBeenCalledTimes(1)
  })
})

describe('downloadStoredAttachment', () => {
  afterEach(() => clearAttachmentCache())

  /* The thumbnail already fetched IS the original; the arrow must not ask
     the store for a second copy. */
  it('hands out the cached bytes without a new request', async () => {
    const trigger = vi.fn(async () => stored('AAAA'))
    await loadAttachment('s-1', 'a_1', trigger)
    const save = vi.fn()
    await downloadStoredAttachment('s-1', 'a_1', 'shot.png', save)
    expect(trigger).toHaveBeenCalledTimes(1)
    expect(save).toHaveBeenCalledTimes(1)
    const [blob, filename] = save.mock.calls[0] as [Blob, string]
    expect(filename).toBe('original.png')
    expect(blob.type).toBe('image/png')
    expect(blob.size).toBe(3)
  })

  it('falls back to the chip name when the store kept none', async () => {
    const trigger = vi.fn(async () => ({
      ...stored('AAAA'),
      attachment: { ...stored().attachment, name: '' },
    }))
    await loadAttachment('s-1', 'a_1', trigger)
    const save = vi.fn()
    await downloadStoredAttachment('s-1', 'a_1', 'shot.png', save)
    expect(save.mock.calls[0][1]).toBe('shot.png')
  })
})
