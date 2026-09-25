#![cfg(target_arch = "wasm32")]

#[test]
fn browser_build_exports_wasm_facade_and_playwright_hooks() {
    // Compile-time API gate. Runtime browser behavior is executed by
    // qa/browser/run-playwright.cjs in Playwright-managed Chromium + Firefox.
    fn assert_export<T>() {}
    assert_export::<p2p_net::wasm::WasmNode>();

    let _ = p2p_net::wasm::qa_storage_write;
    let _ = p2p_net::wasm::qa_storage_read;
    let _ = p2p_net::wasm::qa_profile_lifecycle;
}
