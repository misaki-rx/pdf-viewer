//! Exercise the vendored cache's private algorithm in the normal test suite.
//! Its map only uses operations shared by FxHashMap and the std/no_std maps.
extern crate alloc;

mod sync {
    pub use std::{collections::HashMap as FxHashMap, sync::Arc};
}

#[path = "../vendor/hayro-syntax/src/page/stream_cache.rs"]
mod stream_cache;
