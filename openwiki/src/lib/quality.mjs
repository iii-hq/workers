// Deterministic page quality gate. Adapted from vercel-labs/openwiki: validate a
// generated page with pure functions, then feed the exact failing reasons back
// to the model for a repair pass. This is what turns thin agent output into
// dense, source-grounded pages without an agent loop deciding when it is "done".

// Count words of ACTUAL prose: strip fenced code, inline code, whole "Sources:"
// lines, URLs, and path-like tokens before counting. Defeats padding with
// tables/paths/code so a word target means real explanation.
export function countWords(markdown) {
  const stripped = String(markdown || '')
    .replace(/```[\s\S]*?```/g, ' ')
    .replace(/`[^`\n]+`/g, ' ')
    .replace(/^.*\bSources:\s*.*$/gim, ' ')
    .replace(/https?:\/\/\S+/g, ' ')
    .replace(/[A-Za-z0-9_.-]+\/[A-Za-z0-9_./-]+/g, ' ');
  return (stripped.match(/[A-Za-z0-9][A-Za-z0-9'-]*/g) || []).length;
}

export function countH2(markdown) {
  return (String(markdown || '').match(/^##\s+/gm) || []).length;
}

// Returns a list of human-readable issue strings (empty = passes).
export function getPageQualityIssues(markdown, opts = {}) {
  const minWords = opts.minWords ?? 180;
  const md = String(markdown || '');
  const issues = [];

  if (!/^#\s+.+/m.test(md)) {
    issues.push('Start with a single level-1 heading: "# Page Title".');
  }
  const h2 = countH2(md);
  if (h2 < 3) {
    issues.push(
      `Include at least 3 level-2 sections ("## ..."); found ${h2}. Use sections like Purpose and Scope, Relevant Source Files, System-to-Code Mapping, Execution Flow, Extension Points, Things to Watch.`,
    );
  }
  if (!/^##\s+Relevant Source Files\b/im.test(md)) {
    issues.push('Add a "## Relevant Source Files" section with bullets naming the key files and why each one matters.');
  }
  if (!/Sources:/i.test(md)) {
    issues.push('Ground concrete claims with visible "Sources: path/a.ts, path/b.ts" lines in the prose.');
  }
  const words = countWords(md);
  if (words < minWords) {
    issues.push(
      `Add more explanation: only ${words} words of prose, need at least ${minWords}. Explain what the area does, why it exists, and what to watch when changing it.`,
    );
  }
  return issues;
}

export function pageRepairFeedback(issues) {
  return (
    'Your previous draft did not meet the quality bar. Keep what was accurate and fix ALL of these:\n' +
    issues.map((s) => `- ${s}`).join('\n')
  );
}
