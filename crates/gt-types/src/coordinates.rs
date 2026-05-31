#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Latitude(f64);

impl Latitude {
    pub fn new(degrees: f64) -> Self {
        debug_assert!(
            (-90.0..=90.0).contains(&degrees),
            "latitude out of range: {degrees}"
        );
        Self(degrees)
    }

    pub fn as_degrees(self) -> f64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Longitude(f64);

impl Longitude {
    pub fn new(degrees: f64) -> Self {
        debug_assert!(
            (-180.0..=180.0).contains(&degrees),
            "longitude out of range: {degrees}"
        );
        Self(degrees)
    }

    pub fn as_degrees(self) -> f64 {
        self.0
    }
}
