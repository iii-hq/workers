export function inboxKey(name: string, session_id: string): string {
  return `session/${session_id}/${name}`;
}
