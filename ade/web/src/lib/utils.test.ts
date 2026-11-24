import { expect, test } from 'vitest'
import { cn } from '@/lib/utils'
test('cn keeps the meta size beside an ink colour', () => {
  expect(cn('font-mono text-meta text-ink-faint')).toBe('font-mono text-meta text-ink-faint')
  expect(cn('text-meta', 'text-body')).toBe('text-body')
})
