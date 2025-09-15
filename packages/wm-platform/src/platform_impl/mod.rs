#[path = "windows/mod.rs"]
mod platform;
#[cfg(mac)]
#[path = "macos/mod.rs"]
mod platform;

// #[cfg(detached)]
// #[path = "mock/mod.rs"]
// mod platform;

pub use platform::*;

#[cfg(not(any(win, mac, detached)))]
compile_error!("The platform you're compiling for is not supported. You may need to enable the 'detached' feature.");

#[cfg(detached)]
compile_error!("The 'detached' feature is not yet implemented.");
