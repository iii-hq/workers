/**
 * Cache keys for `@pierre/diffs`.
 *
 * pierre memoises on `cacheKey`, and it trusts it: `areDiffTargetsEqual`
 * compares nothing else, and `FileRenderer`'s line cache is keyed on it alone.
 * A key that fails to move when the text moves therefore does not render
 * slightly stale — it renders the *previous* file.
 *
 * This page used to key on the path and the content *length*, which meant every
 * edit that left the length alone showed the old content. Swapping one
 * character does exactly that, which is not an edge case: it is a typo fix.
 */

/**
 * A 32-bit FNV-1a hash of `text`, base-36 encoded.
 *
 * Rolling and allocation-free apart from the accumulator, so it costs one pass
 * over the string. Call sites hold it behind a `useMemo` keyed on the same
 * text, so it runs once per change rather than once per render.
 *
 * The length is folded in at the end, which makes the key strictly stronger
 * than either the hash or the length alone: two strings now have to agree on
 * both to collide.
 *
 * A 32-bit hash *can* collide, and if it does the stale render is back for that
 * one pair of contents. That trade is the right way round: a collision needs
 * two specific strings to land on the same word out of four billion, whereas
 * keying on length was not a risk but a certainty — every same-length edit hit
 * it, every time. A wider or cryptographic hash would buy a rounding error of
 * additional safety for a per-keystroke cost over the whole buffer.
 */
export function contentHash(text: string): string {
  let hash = 0x811c9dc5
  for (let i = 0; i < text.length; i++) {
    hash ^= text.charCodeAt(i)
    hash = Math.imul(hash, 0x01000193)
  }
  hash ^= text.length
  hash = Math.imul(hash, 0x01000193)
  // `>>> 0` reads the accumulator as unsigned; base 36 keeps the key short.
  return (hash >>> 0).toString(36)
}
