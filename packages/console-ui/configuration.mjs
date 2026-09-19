/** Resolve through the addressed worker's namespace, never a global family lookup. */
export async function resolveConfigurationId(iii, worker) {
  const result = await iii.trigger(`${worker}::configuration-id`, {}, { timeoutMs: 5000 })
  const id = result?.id
  if (typeof id !== 'string' || !id.trim()) {
    throw new Error(`${worker} returned an invalid configuration identity`)
  }
  return id
}
