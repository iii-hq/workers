/* Explorer verbs on files and folders — rename, delete, duplicate,
   clipboard — over the worker's `coder::*` functions. Paths in are
   root-relative; the caller supplies the root. */

import type { Host } from '@iii-dev/console-ui'
import { coderCreateNewFile, coderDelete, coderMove, coderReadFile, coderWriteFile, joinPath, shellCreateFolder } from './coder'

export async function renameEntry(host: Host, root: string, from: string, to: string): Promise<void> {
  if (from === to) return
  await coderMove(host, joinPath(root, from), joinPath(root, to))
}

export async function deleteEntry(host: Host, root: string, rel: string, isDir: boolean): Promise<void> {
  const [result] = await coderDelete(host, [joinPath(root, rel)], isDir)
  if (result && !result.success) {
    throw new Error(result.error?.message ?? `could not delete ${rel}`)
  }
}

export async function createEntry(
  host: Host,
  root: string,
  kind: 'file' | 'folder',
  rel: string,
): Promise<void> {
  const abs = joinPath(root, rel)
  if (kind === 'folder') await shellCreateFolder(host, abs)
  else await coderCreateNewFile(host, abs)
}

/** `a/b.ts` → `a/b copy.ts`, then `a/b copy 2.ts`, … */
export function duplicateName(rel: string, taken: (candidate: string) => boolean): string {
  const slash = rel.lastIndexOf('/')
  const dir = slash === -1 ? '' : rel.slice(0, slash + 1)
  const name = slash === -1 ? rel : rel.slice(slash + 1)
  const dot = name.startsWith('.') ? -1 : name.lastIndexOf('.')
  const stem = dot === -1 ? name : name.slice(0, dot)
  const ext = dot === -1 ? '' : name.slice(dot)
  let candidate = `${dir}${stem} copy${ext}`
  let n = 2
  while (taken(candidate)) {
    candidate = `${dir}${stem} copy ${n}${ext}`
    n += 1
  }
  return candidate
}

export async function duplicateFile(host: Host, root: string, rel: string, to: string): Promise<void> {
  const out = await coderReadFile(host, joinPath(root, rel), { maxOutputBytes: 8 * 1024 * 1024 })
  if (out.is_utf8 === false) throw new Error('binary files cannot be duplicated here')
  const result = await coderWriteFile(host, joinPath(root, to), out.content ?? '', out.mode ?? null)
  if (!result.success) throw new Error(result.error?.message ?? `could not write ${to}`)
}

export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text)
    return true
  } catch {
    return false
  }
}
