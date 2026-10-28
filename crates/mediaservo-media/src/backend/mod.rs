#[cfg(all(feature = "backend-yuv-sys", feature = "backend-native"))]
compile_error!("Only one backend can be enabled at a time.");

#[cfg(feature = "backend-yuv-sys")]
mod libyuv;
#[cfg(feature = "backend-yuv-sys")]
pub use libyuv::LibyuvTransform;
#[cfg(feature = "backend-yuv-sys")]
/// Compile-time alias for the active backend.
pub use libyuv::LibyuvTransform as ActiveTransform;

#[cfg(all(feature = "backend-native", not(feature = "backend-yuv-sys")))]
mod native;
#[cfg(all(feature = "backend-native", not(feature = "backend-yuv-sys")))]
pub use native::NativeTransform;
#[cfg(all(feature = "backend-native", not(feature = "backend-yuv-sys")))]
pub use native::NativeTransform as ActiveTransform;

use crate::transform::VideoTransform;

/// Returns the compiled backend. Zero dynamic dispatch.
#[cfg(feature = "backend-yuv-sys")]
pub fn get_backend() -> impl VideoTransform {
    LibyuvTransform
}

#[cfg(all(feature = "backend-native", not(feature = "backend-yuv-sys")))]
pub fn get_backend() -> impl VideoTransform {
    NativeTransform
}
