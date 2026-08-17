/** Shared truthy env flag parsing for worker options. */
export function envFlag(name) {
  const value = process.env[name]?.trim().toLowerCase();
  return value === 'true' || value === '1' || value === 'yes' || value === 'on';
}
