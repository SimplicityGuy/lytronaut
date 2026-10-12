//! Rust browser application. Domain policies are shared with native tests.
#[cfg(target_arch = "wasm32")]
mod browser;
pub mod gpu;
pub mod model;
#[cfg(target_arch = "wasm32")]
pub mod video;
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    leptos::mount::mount_to_body(browser::app);
}
