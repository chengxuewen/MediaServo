//! MediaServo Media — video frame types, buffers, source/sink pipeline, transforms.

pub mod backend;
pub mod base;
pub mod engine;
pub mod error;
pub mod pipeline;
pub mod pixel_format;
pub mod plugin;
pub mod transform;

#[cfg(not(any(feature = "backend-yuv-sys", feature = "backend-native")))]
compile_error!(
    "At least one backend feature must be enabled: \
     backend-yuv-sys or backend-native"
);
