<!-- partial-info: Optional. Body of the rendered README's `## Migration notes` section when present, omitted entirely when absent. README only. Keep terse — renamed function ids, removed config keys, changed payload shapes. -->

`textstats::analyze` previously returned `{ words, chars }` only. The `lines` field was added in 0.2.0. Callers that relied on the older shape can ignore the new field — it is additive.
