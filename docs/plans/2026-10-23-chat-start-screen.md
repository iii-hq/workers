# Chat start screen implementation plan

**Goal:** Apply the approved compact start-screen concept to the live chat UI.

**Architecture:** Keep one mounted Composer as a session moves from empty to active. Move agent catalog selection and default preselection into a compact composer picker. Preserve worker settings, project/worktree controls, permissions, attachments, draft persistence, and setup/error screens.

**Stack:** React, TypeScript, Tailwind, existing Base UI popover, Lexical, Vitest and Playwright.

1. Add a behavior test for starter prompts: fill and focus only the owning composer, never send, preserve an existing draft. Update the empty-state test to expect a concise heading without setup cards. Run these before implementation.
2. Extract the catalog/default-selection logic from `EmptyState.tsx` into `AgentProfilePicker.tsx`. Preserve frozen profile identities, manual choices, hidden profiles, retry and configuration access. Move the existing preselection timing coverage to the picker and add selection/configuration checks.
3. Update `Composer.tsx`: inline project chip, preserve project extension controls and phone collapse, add one visible agent picker, keep worker controls accessible in agent details, and add locally scoped starter prompts. Give the subtle `ModelPicker` a chevron.
4. Update `ChatView.tsx` to group the empty heading and composer. Keep the composer mounted through the first send; normal conversations retain their bottom composer. Simplify `MessageList.tsx` empty-state wiring and update stories for the new composition.
5. Run focused Vitest tests, TypeScript and formatting checks. Inspect the real components in Storybook at desktop and narrow widths in both themes; verify menu selection, starter insertion, settings access and phone collapse. Save screenshots in `.context`.

Work in the existing Conductor checkout. Preserve all prior uncommitted edits; no branch change or additional worktree is needed. Styling changes are checked visually rather than with class-name snapshot tests.

Completed: implementation, focused behavior tests, browser checks in both themes and narrow widths, production build, and independent review. Verification artifacts are in `.context/start-screen-checks/`; screenshots are `.context/start-screen-implemented-dark.png` and `.context/start-screen-implemented-light.png`. The shorter bundled directory placeholder is source-verified and takes effect after rebuilding/restarting that worker.
