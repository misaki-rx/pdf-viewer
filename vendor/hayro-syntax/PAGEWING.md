# Pagewing decoded-page cache patch

Source: crates.io `hayro-syntax` 0.8.0, https://github.com/LaurenzV/hayro.
Upstream MIT and Apache-2.0 licenses and source are retained.

Local changes:

- Replace each page's permanent `OnceLock<Option<Vec<u8>>>` with a shared,
  per-document LRU in `src/page/stream_cache.rs`: 32 MiB of buffer capacity,
  plus a maximum of 512 entries to bound empty/failed-page bookkeeping.
- Hit promotion and eviction use page-number links with a fixed number of map
  lookups, without scanning or allocating access-order records on hits.
- `Page::page_stream` returns an owned `Arc<Vec<u8>>` snapshot (`Rc` in no_std).
  Eviction drops the cache's reference, while an interpreter or other caller
  can safely finish with its snapshot. Oversized streams bypass admission and
  do not evict useful entries. Decoding runs outside the cache mutex; racing
  readers reuse the first admitted snapshot.
- Reuse already-owned decoder output rather than copying it again.
- Remove the borrowed `Page::operations` / `Page::typed_operations` helpers.
  Callers keep a stream snapshot and construct `UntypedIter::new(&stream)` or
  `TypedIter::new(&stream)`. The interpreter patch and examples follow this API.

Only `src/page.rs`, `src/page/stream_cache.rs`, `src/content/mod.rs`, and the
iteration examples in `src/lib.rs`, `README.md`, and `examples/iterate.rs` differ
from upstream. No new unsafe code or dependencies are required.

Algorithm tests are included by Pagewing's `tests/page_stream_lru.rs` so the normal
`cargo test --locked` runs them without the upstream test suite's external PDF
corpus and dev dependencies. End-to-end tests live in
`src/document/stream_cache_tests.rs`; the benchmark is `benches/page_stream_cache.rs`.

The budget covers retained page-content buffers, not raw PDF bytes, decoded object
streams, font/glyph caches, active snapshots, or decompression working memory.
Remove this patch when upstream provides equivalent bounded, owned page streams.
