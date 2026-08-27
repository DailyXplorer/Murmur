//! Allocator hooks kept as no-ops after dropping Linux glibc `mallopt` /
//! `malloc_trim`. Call sites in `run()` and `FinishGuard` stay unchanged.

pub fn init_allocator() {}

pub fn trim_freed_memory() {}
