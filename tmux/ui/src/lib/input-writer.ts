/**
 * Keyboard input to `tmux::terminal::write`: what is typed while a write is
 * in flight goes out together when it lands — one call per round trip, in
 * order, none above the worker's 64 KiB cap (a bigger paste goes in pieces).
 */

/** The worker refuses a `write` whose decoded data is larger. */
export const MAX_WRITE_BYTES = 64 * 1024

export function createInputWriter(
  send: (data: Uint8Array) => Promise<void>,
  maxBytes = MAX_WRITE_BYTES,
): (data: Uint8Array) => void {
  const pending: Uint8Array[] = []
  let pendingBytes = 0
  let flushing = false
  const flush = async () => {
    flushing = true
    while (pendingBytes > 0) {
      const chunk = new Uint8Array(Math.min(pendingBytes, maxBytes))
      let filled = 0
      while (filled < chunk.length) {
        const head = pending[0]
        const take = Math.min(head.length, chunk.length - filled)
        chunk.set(head.subarray(0, take), filled)
        filled += take
        if (take === head.length) pending.shift()
        else pending[0] = head.subarray(take)
      }
      pendingBytes -= chunk.length
      await send(chunk).catch(() => undefined)
    }
    flushing = false
  }
  return (data) => {
    if (data.byteLength === 0) return
    pending.push(data)
    pendingBytes += data.byteLength
    if (!flushing) void flush()
  }
}

export function encodeBase64(bytes: Uint8Array): string {
  let binary = ''
  for (let offset = 0; offset < bytes.length; offset += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + 0x8000))
  }
  return btoa(binary)
}

export function decodeBase64(value: string): Uint8Array {
  const binary = atob(value)
  const bytes = new Uint8Array(binary.length)
  for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index)
  return bytes
}

/** xterm's `onBinary` hands over one byte per UTF-16 code unit. */
export function binaryStringToBytes(value: string): Uint8Array {
  const bytes = new Uint8Array(value.length)
  for (let index = 0; index < value.length; index += 1) bytes[index] = value.charCodeAt(index) & 0xff
  return bytes
}
