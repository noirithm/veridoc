//! JavaScript-facing wrapper around the analysis engine.

use veridoc_core::CaseBuilder;
use wasm_bindgen::prelude::*;

/// Runs once when the module loads. Without it, a Rust panic shows up in
/// the browser console as an unhelpful "unreachable" error.
#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}

/// A set of documents that are analysed together.
#[wasm_bindgen]
pub struct Case {
    inner: CaseBuilder,
}

impl Default for Case {
    fn default() -> Self {
        Self::new()
    }
}

#[wasm_bindgen]
impl Case {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Case {
        Case {
            inner: CaseBuilder::new(),
        }
    }

    /// Registers a document. `source` is the original file's bytes.
    #[wasm_bindgen(js_name = addDocument)]
    pub fn add_document(&mut self, id: &str, name: &str, source: &[u8]) -> Result<(), JsError> {
        self.inner.add_document(id, name, source).map_err(to_js)
    }

    /// Adds a page image and its recognised text lines (as JSON).
    /// Returns the zero-based page number.
    #[wasm_bindgen(js_name = addPage)]
    pub fn add_page(&mut self, id: &str, image: &[u8], lines_json: &str) -> Result<u32, JsError> {
        self.inner.add_page(id, image, lines_json).map_err(to_js)
    }

    /// Runs the analysis and returns the full report as JSON.
    pub fn analyze(&self) -> Result<String, JsError> {
        self.inner.analyze_json().map_err(to_js)
    }
}

fn to_js(error: veridoc_core::Error) -> JsError {
    JsError::new(&error.to_string())
}
