import { describe, expect, it, vi } from 'vitest'

import type { Attachment } from '@/types/chat'
import {
  expandImageAttachments,
  fitWithin,
  imageMimeOf,
  isImageAttachment,
  MAX_IMAGE_BYTES,
  MAX_IMAGE_EDGE,
  MAX_IMAGES_PER_SEND,
  MAX_SOURCE_IMAGE_BYTES,
  needsDownscale,
} from './images'

function image(name = 'shot.png', type = 'image/png', size = 1024): Attachment {
  const file = new File([new Uint8Array(size)], name, { type })
  return { id: name, name, size, type, file }
}

/** A file that reports a size without allocating it. */
function oversized(name: string, type: string, size: number): Attachment {
  const attachment = image(name, type, 8)
  Object.defineProperty(attachment.file as File, 'size', { value: size })
  return { ...attachment, size }
}

describe('isImageAttachment', () => {
  it('takes the declared type first and the extension second', () => {
    expect(isImageAttachment(image())).toBe(true)
    expect(isImageAttachment(image('photo.HEIC', ''))).toBe(true)
    expect(isImageAttachment(image('notes.txt', 'text/plain'))).toBe(false)
  })
})

describe('imageMimeOf', () => {
  it('normalises jpg to the type providers expect', () => {
    expect(imageMimeOf(image('a.jpg', ''))).toBe('image/jpeg')
    expect(imageMimeOf(image('a.png', 'image/png'))).toBe('image/png')
  })
})

describe('fitWithin', () => {
  it('keeps the aspect ratio and caps the longest edge', () => {
    expect(fitWithin(3200, 1600)).toEqual({
      width: MAX_IMAGE_EDGE,
      height: MAX_IMAGE_EDGE / 2,
    })
  })

  /* Upscaling a small screenshot would add bytes and no detail. */
  it('leaves an image inside the ceiling alone', () => {
    expect(fitWithin(800, 600)).toEqual({ width: 800, height: 600 })
  })
})

describe('needsDownscale', () => {
  it('triggers on the byte ceiling', () => {
    expect(needsDownscale({ size: MAX_IMAGE_BYTES + 1 })).toBe(true)
    expect(needsDownscale({ size: 1024 })).toBe(false)
  })
})

describe('expandImageAttachments', () => {
  const neverDownscales = vi.fn().mockResolvedValue(null)

  it('sends a supported image as a native image block', async () => {
    const result = await expandImageAttachments([image()], neverDownscales)

    expect(result.images).toHaveLength(1)
    expect(result.images[0].type).toBe('image')
    expect(result.images[0].mime).toBe('image/png')
    expect(result.images[0].data.length).toBeGreaterThan(0)
    expect(result.blocks).toEqual([])
    expect(result.failures).toEqual([])
  })

  /* A `.heic` from a phone is a file no provider decodes. A block saying so
     beats an API error the person never sees. */
  it('refuses a format no model reads when it cannot be converted', async () => {
    const result = await expandImageAttachments(
      [image('photo.heic', 'image/heic')],
      neverDownscales,
    )

    expect(result.images).toEqual([])
    expect(result.blocks[0]).toContain('not a format a model can read')
    expect(result.failures).toHaveLength(1)
  })

  it('converts an unsupported format when the browser can', async () => {
    const downscale = vi.fn().mockResolvedValue({
      blob: new Blob([new Uint8Array(64)]),
      mime: 'image/jpeg',
    })
    const result = await expandImageAttachments(
      [image('photo.heic', 'image/heic')],
      downscale,
    )

    expect(downscale).toHaveBeenCalled()
    expect(result.images[0].mime).toBe('image/jpeg')
    expect(result.read[0].label).toContain('resized')
  })

  it('downscales an image over the byte ceiling', async () => {
    const downscale = vi.fn().mockResolvedValue({
      blob: new Blob([new Uint8Array(32)]),
      mime: 'image/jpeg',
    })
    const big = oversized('screenshot.png', 'image/png', MAX_IMAGE_BYTES + 1)
    const result = await expandImageAttachments([big], downscale)

    expect(downscale).toHaveBeenCalledTimes(1)
    expect(result.images).toHaveLength(1)
    expect(result.read[0].label).toContain('resized')
  })

  it('reports an oversized image the browser could not resize', async () => {
    const big = oversized('screenshot.png', 'image/png', MAX_IMAGE_BYTES + 1)
    const result = await expandImageAttachments([big], neverDownscales)

    expect(result.images).toEqual([])
    expect(result.blocks[0]).toContain('could not be resized')
    expect(result.failures).toHaveLength(1)
  })

  /* Past this size the browser stalls the tab decoding it, so nothing is even
     attempted. */
  it('refuses an enormous source file outright', async () => {
    const downscale = vi.fn()
    const enormous = oversized(
      'raw.png',
      'image/png',
      MAX_SOURCE_IMAGE_BYTES + 1,
    )
    const result = await expandImageAttachments([enormous], downscale)

    expect(downscale).not.toHaveBeenCalled()
    expect(result.blocks[0]).toContain('too large to send')
  })

  it('reports the images past the per-send ceiling instead of dropping them', async () => {
    const many = Array.from({ length: MAX_IMAGES_PER_SEND + 1 }, (_, i) =>
      image(`shot-${i}.png`),
    )
    const result = await expandImageAttachments(many, neverDownscales)

    expect(result.images).toHaveLength(MAX_IMAGES_PER_SEND)
    expect(result.failures).toHaveLength(1)
    expect(result.failures[0].reason).toContain('per message')
  })
})
