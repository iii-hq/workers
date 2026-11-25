/**
 * An age one short unit wide, the conversation list's words: `now`, `5m`,
 * `2h`, `3d`, then `2w`, `4mo`, `1y` so an old chat never reads as `143d`.
 * The list rows, the "by agent · 2m" pill and the context pane share it. It is
 * `formatRelative`, the age the palette's chat rows wear, under the list's name.
 */
export { formatRelative as shortAge } from '@/lib/relative-age'
