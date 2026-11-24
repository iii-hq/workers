import { type ClassValue, clsx } from 'clsx'
import { extendTailwindMerge } from 'tailwind-merge'

// `text-meta` and `text-body` are font sizes (theme `--text-*`). Unregistered,
// tailwind-merge reads them as colours and drops them beside `text-ink-faint`.
const twMerge = extendTailwindMerge({
  extend: { classGroups: { 'font-size': [{ text: ['meta', 'body'] }] } },
})

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}
