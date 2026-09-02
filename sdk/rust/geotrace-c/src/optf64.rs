//! The optional double.

/// An optional `double` value. Use the macros below to construct values.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GtdOptF64 {
    pub value: f64,
    pub present: u8,
}

impl GtdOptF64 {
    pub(crate) fn to_opt(self) -> Option<f64> {
        if self.present != 0 {
            Some(self.value)
        } else {
            None
        }
    }
}

pub(crate) fn opt_f64_none() -> GtdOptF64 {
    GtdOptF64 {
        value: 0.0,
        present: 0,
    }
}

pub(crate) fn opt_f64_some(v: f64) -> GtdOptF64 {
    GtdOptF64 {
        value: v,
        present: 1,
    }
}
