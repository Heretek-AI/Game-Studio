#[cfg(feature = "profile")]
pub use tracy_client::span as tracy_span;

#[cfg(feature = "profile")]
#[macro_export]
macro_rules! profile_span {
    ($name:expr) => {
        let _tracy_span = tracy_client::span!($name);
    };
}

#[cfg(not(feature = "profile"))]
#[macro_export]
macro_rules! profile_span {
    ($name:expr) => {
        let _tracy_span: () = ();
        let _ = $name;
    };
}

#[cfg(feature = "profile")]
pub fn emit_frame_mark() {
    tracy_client::frame_mark();
}

#[cfg(not(feature = "profile"))]
pub fn emit_frame_mark() {}

pub const SPAN_SIM: &str = "sim";
pub const SPAN_PHYSICS: &str = "physics";
pub const SPAN_EXTRACT: &str = "extract";
pub const SPAN_AUDIO: &str = "audio";
pub const SPAN_ASSET: &str = "asset";
