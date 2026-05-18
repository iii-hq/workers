export const HANDLER_FN_ID = 'turn::on_terminal_state';
export const CONDITION_FN_ID = 'turn::is_terminal_state_write';

const TURN_STATE_KEY_RE = /^session\/([^/]+)\/turn_state$/;

const pending = new Map<string, () => void>();

export const __pendingForTest = pending;

export function isTerminalStateWrite(event: unknown): boolean {
  if (!event || typeof event !== 'object') return false;
  const obj = event as Record<string, unknown>;
  if (obj.event_type !== 'state:created' && obj.event_type !== 'state:updated') return false;
  const key = obj.key;
  if (typeof key !== 'string' || !TURN_STATE_KEY_RE.test(key)) return false;
  const nv = obj.new_value;
  if (!nv || typeof nv !== 'object') return false;
  return (nv as Record<string, unknown>).state === 'stopped';
}

function extractSessionId(key: string): string | null {
  const m = TURN_STATE_KEY_RE.exec(key);
  return m ? (m[1] ?? null) : null;
}

export function installTerminalWaiter(session_id: string): Promise<void> {
  return new Promise<void>((resolve) => {
    pending.set(session_id, resolve);
  });
}

export function clearTerminalWaiter(session_id: string): void {
  pending.delete(session_id);
}

export function handleTerminalStateWrite(event: unknown): void {
  if (!event || typeof event !== 'object') return;
  const obj = event as Record<string, unknown>;
  const key = obj.key;
  if (typeof key !== 'string') return;
  const session_id = extractSessionId(key);
  if (!session_id) return;
  const resolver = pending.get(session_id);
  if (!resolver) return;
  pending.delete(session_id);
  resolver();
}
