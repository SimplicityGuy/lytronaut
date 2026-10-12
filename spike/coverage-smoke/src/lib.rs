// Throwaway spike (lytronaut-cjq step 1): prove LLVM source-based coverage
// can be captured from Rust/WASM running in headless Chrome via minicov.
use wasm_bindgen::prelude::*;

// HIT-BEGIN
#[wasm_bindgen]
pub fn hit(x: u32) -> u32 {
    let mut acc = 0u32;
    if x > 10 {
        acc += x * 2;
    } else {
        acc += 1;
    }
    for i in 0..x {
        acc = acc.wrapping_add(i);
    }
    acc
}
// HIT-END

// MISS-BEGIN
#[wasm_bindgen]
pub fn miss(x: u32) -> u32 {
    if x == 42 {
        x + 1
    } else {
        x * 7
    }
}
// MISS-END

#[wasm_bindgen]
pub fn cov_dump() -> Vec<u8> {
    let mut v = Vec::new();
    unsafe {
        minicov::capture_coverage(&mut v).expect("capture_coverage failed");
    }
    minicov::reset_coverage();
    v
}

#[wasm_bindgen]
pub fn cov_signature() -> u64 {
    minicov::module_signature()
}
