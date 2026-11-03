// Synthetic-only equivalents of long catalog metadata. No runtime/catalog reads.
export const selectorCatalog = [
  { id: 'synthetic-profile-' + 'id'.repeat(100), name: 'Synthetic specialist ' + 'LongName'.repeat(24), description: 'SyntheticDescription'.repeat(175) },
  { id: 'default', name: 'Default', description: 'Synthetic default profile; select with an ordinary pointer click.' },
  ...Array.from({ length: 12 }, (_, index) => ({ id: `synthetic-extra-${index}`, name: `Synthetic specialist ${index}`, description: ('Synthetic expertise and investigation constraints. ').repeat(30) })),
  { id: 'broken-fixture', name: 'Broken chain', description: 'Synthetic unresolved inheritance', inheritance_error: 'missing synthetic parent' },
]
