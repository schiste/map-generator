//! Transcendental functions from the pure-Rust `libm` crate.
//!
//! `f64::sin` and friends call the platform's math library, whose results can
//! differ in the last bit between macOS, glibc, MSVC and WebAssembly. Using one
//! implementation everywhere keeps output byte-identical on every target.
//! (`sqrt` and basic arithmetic are exactly rounded by IEEE 754, so they are
//! already portable.)

pub fn sin(x: f64) -> f64 {
    libm::sin(x)
}

pub fn cos(x: f64) -> f64 {
    libm::cos(x)
}

pub fn asin(x: f64) -> f64 {
    libm::asin(x)
}

pub fn hypot(x: f64, y: f64) -> f64 {
    libm::hypot(x, y)
}

pub fn atan2(y: f64, x: f64) -> f64 {
    libm::atan2(y, x)
}
