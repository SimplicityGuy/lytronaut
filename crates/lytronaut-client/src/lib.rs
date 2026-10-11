//! Browser application entry point; SQLite and service crates are never dependencies.
#[cfg(target_arch = "wasm32")]
use leptos::prelude::*;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    leptos::mount::mount_to_body(|| {
        view! {
            <main>
                <h1>"Lytronaut"</h1>
                <p>"an explorer for your light-field library"</p>
                <p>"Foundation ready. Library import and browsing are under development."</p>
            </main>
        }
    });
}
