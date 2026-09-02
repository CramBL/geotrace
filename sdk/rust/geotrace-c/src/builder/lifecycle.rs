//! The `builder` group of `geotrace.h`: creating and destroying a builder handle.

use super::GtdFileBuilder;

#[unsafe(no_mangle)]
pub extern "C" fn gtd_builder_create() -> *mut GtdFileBuilder {
    Box::into_raw(Box::new(GtdFileBuilder::new()))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_destroy(b: *mut GtdFileBuilder) {
    if b.is_null() {
        return;
    }
    // SAFETY: b was allocated by `gtd_builder_create` via Box::into_raw
    unsafe { drop(Box::from_raw(b)) };
}
