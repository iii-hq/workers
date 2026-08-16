/** Operator-opt-in governance bypass (unsafe for production). */
export function isBypassMode() {
  const value = process.env.GANTRY_BYPASS_MODE?.trim().toLowerCase();
  return value === 'true' || value === '1' || value === 'yes' || value === 'on';
}
