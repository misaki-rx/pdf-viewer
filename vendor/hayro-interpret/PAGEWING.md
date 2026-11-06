# Pagewing owned page-stream integration

Source: crates.io `hayro-interpret` 0.8.0, https://github.com/LaurenzV/hayro.
Upstream MIT and Apache-2.0 licenses and source are retained.

The only local code change is in `src/interpret/mod.rs`: `interpret_page` holds
the owned page-content snapshot from the patched `hayro-syntax` crate and borrows
it to construct `TypedIter`. This keeps inline image and instruction data alive
for interpretation even if the shared page cache evicts that entry. The public
interpreter and device APIs are unchanged.

See `../hayro-syntax/PAGEWING.md` for cache limits, tests and update guidance.
