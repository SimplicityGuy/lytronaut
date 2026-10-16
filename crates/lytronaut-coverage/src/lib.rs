//! Coverage tooling only. Exposes the LLVM source-based coverage counters of an
//! instrumented `wasm32-unknown-unknown` build to the browser test driver
//! (`scripts/coverage.cjs`), which writes them out as `.profraw` files before
//! every page navigation or close.
//!
//! Linked solely through `lytronaut-client`'s off-by-default `coverage` feature.
//! Native builds of this crate are intentionally empty.

#[cfg(all(target_arch = "wasm32", target_feature = "atomics"))]
compile_error!("coverage capture assumes single-threaded WASM without shared memory");

#[cfg(target_arch = "wasm32")]
mod hook {
    use js_sys::{Object, Reflect, Uint8Array};
    use wasm_bindgen::{JsValue, closure::Closure};

    /// Global object read by `scripts/coverage.cjs`.
    pub const GLOBAL: &str = "__lytronautCoverage";

    /// Serialise the current counters as an LLVM raw profile, then reset them so
    /// a later dump from the same page instance does not repeat earlier hits.
    fn capture() -> Result<Vec<u8>, String> {
        let mut profile = Vec::new();
        // SAFETY: minicov requires that no other thread executes instrumented code
        // while the counters are serialised. This module is only built for
        // wasm32-unknown-unknown without atomics (enforced above), so the browser
        // thread running this synchronous call is the only executor of
        // instrumented code in this module instance.
        unsafe { minicov::capture_coverage(&mut profile) }
            .map_err(|error| format!("LLVM profile capture failed: {error:?}"))?;
        minicov::reset_coverage();
        Ok(profile)
    }

    /// Publish `{ dump(): Uint8Array, signature(): string }` on the global object.
    /// The signature is a decimal string to avoid JS number precision loss.
    pub fn install() {
        let hook = Object::new();
        let dump = Closure::<dyn Fn() -> Result<Uint8Array, JsValue>>::new(|| {
            capture()
                .map(|profile| Uint8Array::from(profile.as_slice()))
                .map_err(|error| JsValue::from_str(&error))
        });
        let signature =
            Closure::<dyn Fn() -> String>::new(|| minicov::module_signature().to_string());
        let published = Reflect::set(&hook, &"dump".into(), dump.as_ref())
            .and_then(|_| Reflect::set(&hook, &"signature".into(), signature.as_ref()))
            .and_then(|_| Reflect::set(&js_sys::global(), &GLOBAL.into(), &hook));
        // The hook lives for the whole page; the driver fails loudly if absent.
        dump.forget();
        signature.forget();
        if published.is_err() {
            wasm_bindgen::throw_str("coverage hook could not be published");
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use hook::install;
