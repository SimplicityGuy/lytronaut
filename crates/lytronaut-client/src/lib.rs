//! Rust browser application. Domain policies are shared with native tests.
#[cfg(target_arch = "wasm32")]
mod browser;
pub mod gpu;
pub mod model;
#[cfg(all(feature = "test-harness", any(target_arch = "wasm32", test)))]
mod retained_observations;
#[cfg(target_arch = "wasm32")]
pub mod video;
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    #[cfg(not(feature = "test-harness"))]
    leptos::mount::mount_to_body(browser::app);
    #[cfg(feature = "test-harness")]
    harness_remount();
}

#[cfg(all(target_arch = "wasm32", feature = "test-harness"))]
type TestUnmount = Option<Box<dyn FnOnce()>>;

#[cfg(all(target_arch = "wasm32", feature = "test-harness"))]
thread_local! {
    static TEST_MOUNT: std::cell::RefCell<TestUnmount> = const {
        std::cell::RefCell::new(None)
    };
}

/// Test-only: dispose the real mounted view and reactive owner.
#[cfg(all(target_arch = "wasm32", feature = "test-harness"))]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn harness_dispose() {
    let unmount = TEST_MOUNT.with(|slot| slot.borrow_mut().take());
    if let Some(unmount) = unmount {
        unmount();
    }
}

/// Test-only: mount the unchanged application under a fresh reactive owner.
#[cfg(all(target_arch = "wasm32", feature = "test-harness"))]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn harness_remount() {
    harness_dispose();
    let parent = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.body())
        .expect("test document body");
    let handle = leptos::mount::mount_to(parent, browser::app);
    TEST_MOUNT.with(|slot| *slot.borrow_mut() = Some(Box::new(move || drop(handle))));
}

/// Test-only synchronous admission observation; never starts a native request.
#[cfg(all(target_arch = "wasm32", feature = "test-harness"))]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn harness_gpu_acquisition_pending() -> bool {
    gpu::acquisition_pending()
}
