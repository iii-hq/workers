/* The icon beside a file name: the same mark in the explorer, the tabs, the
   breadcrumbs, search, source control and the timeline.

   Marks come from material-icon-theme (the set VS Code's Material Icon
   Theme draws): the real HTML, CSS, TypeScript, YAML, Rust, Docker … logos,
   bundled as one sprite by scripts/file-icons.mjs with the file-name and
   extension tables that pick them. The explorer tree (a shadow DOM) gets
   the same sprite and tables through @pierre/trees' icon config, so a file
   wears one icon everywhere. Anything the bundled set does not cover falls
   back to @pierre/trees' built-in "complete" set, with a few of its blank
   glyphs replaced by a plain picture of the file. */

import { createFileTreeIconResolver, type FileTreeIconConfig, getBuiltInSpriteSheet } from '@pierre/trees'
import { File, FileCog, FileLock2, FileText, type LucideIcon } from 'lucide-react'
import { useEffect } from 'react'
import { FILE_ICON_BY_EXT, FILE_ICON_BY_NAME, FILE_ICON_SPRITE } from './file-icons.generated'

const ICON_SET = 'complete'
const SPRITE_HOST_ID = 'shui-file-type-sprite'
const BUILTIN_PREFIX = 'file-tree-builtin-'
const FILE_SLOT = 'file-tree-icon-file'
const MATERIAL_PREFIX = 'mi-'

const resolver = createFileTreeIconResolver({ set: ICON_SET, colored: true })

/** The material icon id for a path, by file name first, then by its longest
    matching extension ("test.ts" before "ts"); null when none is bundled. */
export function materialIconFor(path: string): string | null {
  const name = path.slice(path.lastIndexOf('/') + 1).toLowerCase()
  if (name === '') return null
  const byName = FILE_ICON_BY_NAME[name]
  if (byName) return byName
  const parts = name.split('.')
  for (let i = 1; i < parts.length; i += 1) {
    const hit = FILE_ICON_BY_EXT[parts.slice(i).join('.')]
    if (hit) return hit
  }
  return null
}

function remap(id: string) {
  return { name: `${MATERIAL_PREFIX}${id}`, width: 16, height: 16, viewBox: '0 0 16 16' }
}

/** The explorer's icon config: the material sprite and its tables over the
    built-in set. The library lower-cases names before matching. */
export const TREE_ICONS: FileTreeIconConfig = {
  set: ICON_SET,
  colored: true,
  spriteSheet: FILE_ICON_SPRITE,
  byFileName: Object.fromEntries(Object.entries(FILE_ICON_BY_NAME).map(([name, id]) => [name, remap(id)])),
  byFileExtension: {
    // Plain text files the built-in set leaves blank: a log, a pid file.
    pid: 'file-tree-builtin-text',
    txt: 'file-tree-builtin-text',
    ...Object.fromEntries(Object.entries(FILE_ICON_BY_EXT).map(([ext, id]) => [ext, remap(id)])),
  },
}

const CONFIG_EXTENSIONS = new Set(['ini', 'conf', 'cfg', 'properties'])
const TEXT_EXTENSIONS = new Set(['txt', 'text', 'pid', 'out'])

function extensionOf(path: string): string {
  const name = path.slice(path.lastIndexOf('/') + 1).toLowerCase()
  const dot = name.lastIndexOf('.')
  return dot > 0 ? name.slice(dot + 1) : ''
}

/** The plain icon a path wears when neither set has a mark for it. */
function plainIcon(path: string, token: string): { Icon: LucideIcon; token: string } | null {
  const ext = extensionOf(path)
  if (ext === 'lock' || path.endsWith('.lockb')) return { Icon: FileLock2, token: 'default' }
  if (CONFIG_EXTENSIONS.has(ext)) return { Icon: FileCog, token: 'yml' }
  if (token === 'text' || TEXT_EXTENSIONS.has(ext)) return { Icon: FileText, token: 'default' }
  if (token === 'default') return { Icon: File, token: 'default' }
  return null
}

/** The built-in set's token for a path (the tone the tabs and crumbs tint by). */
export function fileTypeToken(path: string): string {
  const name = resolver.resolveIcon(FILE_SLOT, path).name
  return name.startsWith(BUILTIN_PREFIX) ? name.slice(BUILTIN_PREFIX.length) : 'default'
}

function ensureSprite(): void {
  if (typeof document === 'undefined' || document.getElementById(SPRITE_HOST_ID)) return
  const host = document.createElement('div')
  host.id = SPRITE_HOST_ID
  host.hidden = true
  host.innerHTML = getBuiltInSpriteSheet(ICON_SET) + FILE_ICON_SPRITE
  document.body.append(host)
}

export function FileTypeIcon({ path, className }: { path: string; className?: string }) {
  useEffect(ensureSprite, [])
  const classes = className ? `shui-file-type-icon ${className}` : 'shui-file-type-icon'
  const material = materialIconFor(path)
  if (material !== null) {
    return (
      // A <use> of the material sprite, not a hand-drawn glyph. lint-allow no-inline-svg
      <svg className={classes} data-token="material" viewBox="0 0 16 16" width={16} height={16} aria-hidden="true" focusable="false">
        <use href={`#${MATERIAL_PREFIX}${material}`} />
      </svg>
    )
  }
  const token = fileTypeToken(path)
  const plain = plainIcon(path, token)
  if (plain) {
    const { Icon } = plain
    return <Icon className={classes} data-token={plain.token} data-plain="" aria-hidden focusable="false" />
  }
  const icon = resolver.resolveIcon(FILE_SLOT, path)
  return (
    // The icon is a <use> of @pierre/trees' sprite, not a hand-drawn glyph. lint-allow no-inline-svg
    <svg
      className={classes}
      data-token={token}
      viewBox={icon.viewBox ?? '0 0 16 16'}
      width={icon.width ?? 16}
      height={icon.height ?? 16}
      aria-hidden="true"
      focusable="false"
    >
      <use href={`#${icon.name}`} />
    </svg>
  )
}
