/**
 * Display names for the catalogue's machine words. Families, states and
 * delivery modes are lowercase identifiers in the engine; the interface
 * reads them in sentence case, with HTTP as the one acronym.
 */

import type { FamilySpec } from './observer-kinds'

export function sentence(word: string): string {
  if (!word) return word
  return word[0].toUpperCase() + word.slice(1)
}

/** `http` → `HTTP`, `schedule` → `Schedule`, `ADE asset` stays. */
export function familyName(spec: Pick<FamilySpec, 'label'>): string {
  if (spec.label === 'http') return 'HTTP'
  return sentence(spec.label)
}
