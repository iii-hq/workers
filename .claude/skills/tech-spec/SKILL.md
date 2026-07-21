---
name: tech-spec
description: Design rigorous, implementable technical specifications, design documents, RFCs, and architecture specs for non-trivial features, services, protocols, systems, integrations, data models, or migrations. Use before or alongside implementation to ground a design in existing contracts, architecture, conventions, and prior art. Do not use for implementing an already-approved spec, writing a lightweight proposal or ticket, answering conversationally, or performing research without a specification deliverable.
---

# Tech Spec

Produce a research-grounded specification that is honest about existing work, precise enough to implement, and adversarially reviewed before delivery.

This skill is self-contained; load no companion files.

## Progress updates

Post a short, one-line status before each phase, such as "scouting the existing system," "fanning out research," "writing the spec," or "reviewing." Keep long-running spec work visibly active.

## Core workflow

Follow all five phases. Never skip Phase 1 (grounding) or Phase 4 (verification). Writing from memory and trusting the first draft are the default failure modes.

### Phase 0 — Frame

1. Restate the goal in one sentence.
2. Identify the single load-bearing constraint: the platform limitation, existing contract, or system property that forces most of the design. Organize the specification around it.
3. Choose the deliverable shape:
   - Use one document for a cohesive design.
   - Use an index plus per-topic files when the design has three or more distinct concerns.
4. Find a sibling specification and mirror the destination's structure, section names, and depth.

### Phase 1 — Scout, then research in parallel

Establish what is true before writing the specification.

1. Scout inline to map the work: identify the existing files, systems, types, and conventions in scope. Do not parallelize research until this map exists.
2. Research these dimensions in parallel when subagents or parallel workflows are available; otherwise research them sequentially:
   - **Foundation or mirror:** Read the existing or adjacent implementation on which the design builds. Do not guess.
   - **Contracts and exact shapes:** Capture request and response types, message formats, schemas, and error shapes verbatim, including exact field names and types.
   - **House conventions:** Identify the destination's documentation style, structure, and required sections.
   - **Integration patterns:** Trace configuration, dependencies, lifecycle, build and deployment behavior, and testing patterns.
   - **Prior art and in-flight work:** Check whether the capability already exists or a contradictory effort is underway.
3. Ground every factual claim in source using `file:line` references and verbatim definitions where appropriate.
4. Never invent a contract. Mark every type, field, error code, or response shape that does not exist yet as a proposed new shape.

### Phase 2 — Run a completeness critique

Try to break the premise before drafting.

1. List everything missing, uncertain, or contradictory that could block a correct specification.
2. Check the premise:
   - Determine whether the capability already exists.
   - Determine whether it conflicts with in-flight work.
   - Distinguish genuinely new behavior from re-homed, borrowed, or duplicated behavior.
3. If the premise changes whether the specification should exist or how it must be framed, surface that finding to the user before writing. Do not claim false novelty or quietly design a duplicate.
4. Resolve open design decisions with sensible defaults. Ask the user only for genuine judgment calls whose answers materially change the design.

### Phase 3 — Write one coherent artifact

Write the final artifact yourself in the destination's house style; use delegated research as evidence rather than stitching together independently authored sections. Include the following:

- **Honest framing:** Explain why the design exists, how it relates to adjacent or existing solutions, and what is new, re-homed, or borrowed. Lead with the load-bearing constraint.
- **Grounded component contracts:** Use exact real types rather than prose approximations. Label proposed contracts explicitly.
- **Diagrams and tables:** Include a sequence or flow diagram for the lifecycle. Use tables for decision rules, configuration fields, and type references.
- **Boundaries:** State non-goals, dependencies, and, when relevant, the exposure, threat, and failure surfaces.
- **Intentional divergences:** Name every deliberate difference from an implementation for which the specification claims parity or fidelity. Treat undisclosed divergence presented as parity as a defect.
- **Implementability:** Make every rule satisfiable from the specification alone. When a rule needs data, identify its source. Never say "filter X by Y" unless the design shows where Y comes from.
- **Navigation:** For a multi-file specification, add an index and resolving cross-references.

### Phase 4 — Review adversarially and verify

Review the specification across at least these dimensions:

1. Requirements coverage.
2. Technical and contract correctness.
3. Fidelity to any implementation or system it mirrors.
4. Internal coherence, including cross-references, terminology, and counts.

Default to refuting each review finding. Verify it against source before acting; many apparent findings are misreadings of the specification or code. Apply only confirmed findings.

After applying confirmed findings, run a consistency sweep:

- Make counts and terminology consistent across all files.
- Resolve every anchor and cross-link.
- Remove stale claims left behind by edits.
- Recheck cited `file:line` references against the final source.

## Completion criteria

Deliver the specification only when it is:

- **Grounded:** Contracts and shapes match reality and include source references.
- **Implementable:** The document is sufficient to build the design; no rule depends on unstated data.
- **Honest:** Existing work, deliberate divergences, and open questions are explicit.
- **Bounded:** Non-goals and dependencies are explicit.
- **Coherent:** Terminology and counts are consistent, and every cross-reference resolves.

## Anti-patterns

Reject any draft that:

- Relies on memory instead of grounding contracts in source.
- Invents request or response shapes, field names, or error codes without marking them as proposed.
- Claims parity or fidelity while silently diverging.
- Filters or transforms data without specifying how to obtain that data.
- Asserts novelty without checking existing capabilities and in-flight work.
- Ships without an adversarial pass that verifies findings against source.
- Leaves inconsistent counts, names, anchors, or cross-references after editing a multi-file specification.
