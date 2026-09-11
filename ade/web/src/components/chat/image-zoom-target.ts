/**
 * The transcript treats every substantial image in a message as viewable:
 * one click opens it in the console's image viewer, whichever worker's
 * renderer produced it. Delegation from the list container means an
 * injected UI never has to wire this up — and never gets it subtly wrong.
 */

/** Icons and avatars stay plain; anything at least this big is content. */
const MIN_CONTENT_IMAGE_PX = 48

/** The slice of an image element this module reads, structurally, so the
    decision is testable without a DOM. */
type ImageLike = {
  tagName?: string
  alt?: string
  src?: string
  currentSrc?: string
  closest?: (selector: string) => unknown
  getBoundingClientRect?: () => { width: number; height: number }
}

/**
 * The image a click should open in the viewer, or null when the click is
 * not on a viewable image: not an image at all, an image that is itself
 * part of a control (a link, a button, an opted-out region), or one too
 * small to be content.
 */
export function imageZoomTarget(
  target: EventTarget | null,
): { src: string; alt: string } | null {
  const image = target as ImageLike | null
  if (!image || image.tagName !== 'IMG') return null
  if (
    typeof image.closest === 'function' &&
    image.closest('a[href], button, [role="button"], [data-image-zoom-exempt]')
  ) {
    return null
  }
  const rect =
    typeof image.getBoundingClientRect === 'function'
      ? image.getBoundingClientRect()
      : null
  if (
    !rect ||
    rect.width < MIN_CONTENT_IMAGE_PX ||
    rect.height < MIN_CONTENT_IMAGE_PX
  ) {
    return null
  }
  const src = image.currentSrc || image.src
  if (!src) return null
  return { src, alt: image.alt || 'image' }
}
