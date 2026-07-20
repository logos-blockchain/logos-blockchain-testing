# Book Maintenance Guide

Working rules for editing the book. Read this before any book change: layout,
wording, new sections, diagrams. It consolidates the conventions the rewrite
was built under so later sessions do not rediscover them.

## Ground truth and workflow

- The book lives in `book/` on this repository's main line (`master`/`dev`).
  There is no separate book branch; edit here. A leftover worktree
  (`../nomos-testing-book`, branch `book-rewrite`) may still exist — it is
  retired; do not edit there.
- Build with `mdbook build book`; preview with `mdbook serve book` (default
  port 3000). Output goes to `target/book/`.
- Deploys: pushing `master` with `book/**` changes publishes to GitHub Pages
  automatically. `dev` pushes do not deploy.
- Commits: title only, no body, no bullets. The taplo pre-commit hook fails
  offline (schema catalog fetch) — commit with `--no-verify` when it does.
  Never push.

## Voice (the most-corrected area; follow strictly)

Factual reference voice, like a senior engineer's internal wiki page. Andrus
has rejected drafts twice over this.

- No marketing or lecture language: no "powerful", "seamless", taglines,
  aphorisms ("teardown design is exposure design"), rhetorical questions, or
  keynote framing. Headings name topics, not hooks.
- No maturity inflation: an uncommitted experiment is a "prototype
  integration", never a "real adopter" or "production".
- No LLM-typical phrasing. Em-dashes at most ~1 per paragraph (table cells and
  nav lists exempt). Every sentence has a finite main verb — the banned
  fingerprint is the verbless colon-fronted appositive with participial tails:
  "A compact mini-book: the framework's main concepts in one coherent read,
  each section linking to its full chapter." Write instead: "This page
  summarizes the framework's main concepts. Each section links to a chapter
  that covers its topic in full." No "X is not Y — it is Z" setup-payoff, no
  habitual triads, no coined metaphors (machinery, plumbing, "the X story"),
  no glossy compressions ("at a glance", "in one coherent read").
- Established terms are fixed: entry pattern, imperative side door, ownership
  mode, handle, run window, cooldown, exposure order. "Deployer" is the
  backend; "Runner" is what `deploy()` returns — never mix.

## Chapter template

H1, then a one-sentence summary with a main verb. `---` between H2 sections.
Title Case headings. `**Note:**` / `**Important:**` bold admonitions;
blockquotes only for external-project callouts, labeled `> **External
example:**`. Code fences: `rust,ignore` / `bash` / `mermaid` / `text`.
Cross-links as relative `[Title](file.md)`. External projects (currently only
logos-blockchain) appear only in the labeled callouts; everything else teaches
through the in-repo example apps. LEZ callouts were removed 2026-07-20 as
outdated — do not re-add them. Chapters run roughly 80–250 lines.

## The Brief (framework-in-brief.md) specifics

Presentation-styled page scoped under `<div class="tour">`. Its CSS lives in
`book/theme/tour-v2.css`, its JS (section rail, folds, map zoom) in
`book/theme/tour-v2.js`; both are registered in `book/book.toml`. Available classes:
`lead` (opening paragraph of a section), `unpacks` (chip under a heading),
`recap` (strip between sections), `seq` (arrow strip), `facts` (label:value
grid), `spine` (the one-sentence glossary relation; plain `<b>` renders as a
neutral chip), `gcards` (three glossary pair cards; each `gcard` holds a
`gcard-label` and `gterm` rows of inline SVG icon + term + `ggloss` phrase,
stacks to one column under 640px), `gloss` (term | short gloss | definition
grid with `g-term` / `g-gloss` / `g-def` cells and `brk` pair separators,
collapses to two columns under 640px; lives inside the "full definitions"
fold), `duo` (two cards), `code-notes` (①②③ list matching code markers),
`tk tk-cluster|tk-process|tk-handle|tk-scenario` (concept chips), `details`
styling. Concept hues used everywhere (chips, mermaid classDefs, code
accents): cluster `#4a90d9`, process `#e08a3c`, handle `#4caf7d`,
scenario/runtime `#9b6dd6`. Mermaid edges are forced visible book-wide via
`tour.css`; markdown inside raw HTML blocks is not processed — use `<code>`
inside `.facts`/`.seq` divs, never backticks. The page is a slide deck. Slide
anatomy classes: `slide` (the panel), `slide--top` (standalone hand-authored
panels at the page top: the framework, six terms, the whole test, the DSL),
`slide-kick` (small kicker), `slide-line` (one-sentence headline),
`slide-note` (small caption), `nodes`/`nd`/`ndw`/`nd-tag`/`nda` (concept-chip
flow diagrams; `nd-cluster|process|handle|scenario` hue variants),
`tiles`/`tile` (alternative rows; same hue variants plus `tile--dash`/
`tile--dot` border styles encoding attached/external). Visual hierarchy rule:
hued nodes/tiles carry a ~10% tint fill (solidity = the concept to look at;
dashed/dotted variants stay hollow — the tint drains as framework ownership
decreases); enumeration rows take `nodes--list` (small hollow chips) so only
true flows read as flows. Sections: `tour-v2.js`
turns every `## N ·` heading followed by an `.unpacks` chip into a `.sec`
slide card — heading restyled as kicker, chip hidden (still required as the
deck gate), corner `.sec-num` numeral, the authored `.slide` visible, and the
body collapsed via height:0 until clicked; an expand-all control sits above
the deck, anchor navigation (rail, "section N" links, search) opens the
target section, and print forces everything open. Every section needs both
an `.unpacks` chip and a `.slide` (headline + nodes or tiles + note). Sections are numbered "N ·" and
cross-referenced as "section N" — renumber ALL references when inserting a
section (previous miss: a capitalized "Section N" escaped a lowercase-only
sweep).

## Accuracy discipline (drift is the #1 recurring failure)

The codebase moves fast; quoted code rots in days. Rules:

- Document nothing before it lands on the main line. Aspirational API lives
  only in `docs/*-plan.md` files as labeled targets.
- Verify every API name against the current source before writing it. For
  dense snippets, compile them (temp bin under an example crate, then delete —
  this caught real bugs twice).
- Prefer quoting from tested code: acceptance tests (`multi-app-e2e`,
  `queue-e2e`) and example bins are the source for snippets. State the run
  command with each quoted example.
- After code lands that the book mentions, run a sync sweep: grep the book for
  the old names; check `framework-in-brief.md`, `running-examples.md`,
  `crate-map.md`, `troubleshooting.md`, `composing-stacks.md` first — they
  concentrate cross-references.

## Verification routine before committing a book round

1. `mdbook build book` passes.
2. Link check: every `(*.md)` target in SUMMARY and chapters exists.
3. Banned-token sweep: stale example names, dead API names, "Adopter note",
   marketing words, first-person headings.
4. If diagrams changed: load the page in the browser preview and confirm every
   mermaid block rendered to SVG (parse errors fail silently to raw text);
   check light and dark themes for new colors.
5. If snippets changed: compile-check them.

## Current known state (2026-07-20; re-verify, do not trust blindly)

- The Brief uses the hand-authored `framework-map.svg`, a section deck, and
  the queue verb DSL as its compact worked example.
- The full book documents the verb layer in `verb-layer.md` and shared app
  cluster provisioning in `cluster-provisioning.md`.
- App handles are access surfaces. Managed app lifetime belongs to the LIFO
  cleanup stack; do not reintroduce clone-count ownership language.
- App-layer provisioning has a backend seam, but the only implementation that
  starts composed child resources today is local. Keep Compose and Kubernetes
  claims aligned with `app-backend-scope.md` and `capability-matrix.md`.
