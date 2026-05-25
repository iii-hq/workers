export function inboxKey(name: string, session_id: string): string {
  return `${session_id}/${name}`;
}
