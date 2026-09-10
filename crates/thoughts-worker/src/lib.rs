//! Thin browser adapter. DOM, events and focus stay in JavaScript.
#![cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg(not(feature = "airports"))]
#[wasm_bindgen]
pub fn crop_number(value: f64) -> String {
    thoughts_core::crop::format_number(value)
}

#[cfg(not(feature = "airports"))]
#[wasm_bindgen]
pub fn crop_calculation(yield_kg: f64, rate: f64, crop_kg: f64) -> Result<String, JsError> {
    if [yield_kg, rate, crop_kg]
        .iter()
        .any(|v| !v.is_finite() || *v <= 0.0)
    {
        return Err(JsError::new(
            "Crop receipt inputs must be positive and finite.",
        ));
    }
    serde_json::to_string(&thoughts_core::crop::Receipt::new(yield_kg, rate, crop_kg))
        .map_err(|e| JsError::new(&e.to_string()))
}

#[cfg(feature = "airports")]
#[wasm_bindgen]
pub fn airport_search(query: &str) -> Result<String, JsError> {
    serde_json::to_string(&thoughts_core::airports::search_airports(query, 8))
        .map_err(|e| JsError::new(&e.to_string()))
}
