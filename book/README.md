# Testing Framework Book

The book is an mdBook rooted at `book/src/SUMMARY.md`. Its current editing conventions and verification checklist live in [`../docs/book-maintenance.md`](../docs/book-maintenance.md).

Build and test it from the repository root:

```bash
mdbook build book
mdbook test book
```

Preview it locally with:

```bash
mdbook serve book
```

Generated output goes to `target/book/`.
