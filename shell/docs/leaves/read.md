# Streaming a file's bytes through a channel

## When to use

- A peer worker wants the bytes of a file without an inline copy in the trigger response.
- Streaming a large file into another channel — for example, uploading into a sandbox — without pinning a buffer in the orchestrator.
- Reading a binary that would not survive JSON encoding inline.

## Notes

- Most LLM tool surfaces want bytes inlined into a tool result rather than a channel handle. For the harness web surface, the inline wrapper `harness::fs::read_inline` drives `shell::fs::read`, drains the channel, and returns the legacy `{ content: [{ text }], details: { size, truncated, bytes_read } }` envelope. Use the wrapper from the browser; reach for `shell::fs::read` directly only when you actually want the streaming channel.
- When `cfg.fs.max_read_bytes > 0` and the file's size exceeds it, the read is rejected before any bytes flow with `S218 file size N exceeds max_read_bytes M`. The default of `0` means no cap, which is unusual — operators typically pin a value.
- Same jail and denylist rules as `shell::fs::ls`.
