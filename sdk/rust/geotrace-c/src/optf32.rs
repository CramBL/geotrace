//! The optional float.

/// An optional `float` value. Use the `GTD_SOME_F32` and `GTD_NONE_F32` macros
/// to construct values.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GtdOptF32 {
    pub value: f32,
    pub present: u8,
}

impl GtdOptF32 {
    pub(crate) fn to_opt(self) -> Option<f32> {
        if self.present != 0 {
            Some(self.value)
        } else {
            None
        }
    }
}

pub(crate) fn opt_f32_none() -> GtdOptF32 {
    GtdOptF32 {
        value: 0.0,
        present: 0,
    }
}

pub(crate) fn opt_f32_some(v: f32) -> GtdOptF32 {
    GtdOptF32 {
        value: v,
        present: 1,
    }
}
