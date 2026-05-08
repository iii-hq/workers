# Sizing text before provider calls

## When to use

- Sizing a prompt before sending it to a provider with a token-budget gate.
- Detecting empty or near-empty user input as an early-out before doing heavier work.
- Recording one stat row per analysis to feed `textstats::summarize` rollups.

## Notes

- `chars` counts unicode scalar values, so a multi-byte emoji counts as one character even though it occupies several bytes on the wire.
- `lines` counts line breaks; an empty string returns `lines: 0`.
- Each call fires `textstats::on-analyze` for any worker subscribing to it.
