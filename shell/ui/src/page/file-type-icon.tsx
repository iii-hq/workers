import { createFileTreeIconResolver, getBuiltInSpriteSheet } from '@pierre/trees'
import { useEffect } from 'react'

const ICON_SET = 'complete'
const SPRITE_HOST_ID = 'shui-file-type-sprite'
const BUILTIN_PREFIX = 'file-tree-builtin-'
const FILE_SLOT = 'file-tree-icon-file'

const resolver = createFileTreeIconResolver({ set: ICON_SET, colored: true })

export function fileTypeToken(path: string): string {
  const name = resolver.resolveIcon(FILE_SLOT, path).name
  return name.startsWith(BUILTIN_PREFIX) ? name.slice(BUILTIN_PREFIX.length) : 'default'
}

function ensureSprite(): void {
  if (typeof document === 'undefined' || document.getElementById(SPRITE_HOST_ID)) return
  const host = document.createElement('div')
  host.id = SPRITE_HOST_ID
  host.hidden = true
  host.innerHTML = getBuiltInSpriteSheet(ICON_SET)
  document.body.append(host)
}

export function FileTypeIcon({ path, className }: { path: string; className?: string }) {
  useEffect(ensureSprite, [])
  const icon = resolver.resolveIcon(FILE_SLOT, path)
  return (
    <svg
      className={className ? `shui-file-type-icon ${className}` : 'shui-file-type-icon'}
      data-token={fileTypeToken(path)}
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
