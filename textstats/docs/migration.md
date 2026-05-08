`textstats::analyze` previously returned `{ words, chars }` only. The `lines` field was added in 0.2.0. Callers that relied on the older shape can ignore the new field — it is additive.
