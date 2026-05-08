# shell::fs::read

Open a stream channel and return the bytes of a file via that channel — NOT inline content.

`({ path, target? }) → { content: ContentRef, size, mode, mtime }` — `content` is a `ContentRef` (`{ channel_id, access_key, direction: "read" }`) that mirrors the SDK's `StreamChannelRef`. The caller drains the channel until close. Total bytes are bounded by `cfg.fs.max_read_bytes` (default `0` = unbounded).

## When to use

- A peer worker wants to consume a file's bytes without an inline copy in the trigger response.
- Streaming large files into another channel (e.g. uploading to a sandbox) without pinning a buffer in the orchestrator.
- Reading a binary that wouldn't survive JSON encoding inline.

## Notes

- Most LLM tool surfaces want bytes inlined into a tool result, NOT a channel handle. For the harness web surface, the inline wrapper [`harness::fs::read_inline`](iii://fn/harness/fs/read_inline) drives `shell::fs::read`, drains the channel, and returns the legacy `{ content: [{ text }], details: { size, truncated, bytes_read } }` envelope. Use the wrapper from the browser; reach for `shell::fs::read` directly only when you actually want the streaming channel.
- When `cfg.fs.max_read_bytes > 0` and the file's size exceeds it, the read is rejected before any bytes flow with `S218 file size N exceeds max_read_bytes M` (mirrors the `S218` write-side cap). The default of `0` means no cap; this is unusual and operators typically pin a value.
- Same jail + denylist rules as `shell::fs::ls`.
