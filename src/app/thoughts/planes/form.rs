//! The flight form and airport combobox: ports of `FlightForm.tsx` and
//! `AirportCombobox.tsx` from ~/how-bad.
//!
//! The baseline is a plain GET form submitting to the page's own URL: the
//! route fields are text inputs named `from`/`to` holding IATA codes, so the
//! whole flow works with JavaScript disabled. The combobox is a client-side
//! enhancement (`airport-combobox.js`): it fetches the bundled airports
//! dataset once and runs the same search (prefix, fuzzy, metros, country)
//! in the browser — no shard round-trips.

use topcoat::{
    Result,
    asset::{Asset, asset},
    view::{component, view},
};

use super::{airports::Airport, emissions::Cabin};
use crate::components::stamp_seal;

const AIRPORT_COMBOBOX_JS: Asset = asset!("./airport-combobox.js");
const AIRPORTS_JSON: Asset = asset!("../../../../data/airports.json");

/// One rendered via field: the filled layovers, plus one empty slot. Every
/// input shares `name="via"` (the server reads the repeated params in
/// order), so only the id and label are numbered.
struct ViaField {
    label: String,
    id: String,
    iata: String,
    hint: String,
}

fn via_field(i: usize, iata: String, hint: String) -> ViaField {
    ViaField {
        label: if i == 0 {
            "Via".to_string()
        } else {
            format!("Via {}", i + 1)
        },
        id: format!("airport-via{}", i + 1),
        iata,
        hint,
    }
}

/// `total` is the filed route's CO₂e figure (e.g. "1.2 t"), stamped on the
/// docked strip; pass `""` when there's nothing to stamp yet.
///
/// `vias` are the route's resolved layovers, in order. The form shows them
/// all, plus one empty via slot — so a route grows a stop per plain form
/// submit, however many stops it already has. No JS required.
#[component]
pub async fn flight_form(
    from: Option<Airport>,
    to: Option<Airport>,
    vias: Vec<Airport>,
    cabin: Cabin,
    round_trip: bool,
    revealed: bool,
    #[default(String::new())] total: String,
) -> Result {
    let mut via_fields: Vec<ViaField> = vias
        .iter()
        .enumerate()
        .map(|(i, a)| via_field(i, a.iata.clone(), String::new()))
        .collect();
    via_fields.push(via_field(
        via_fields.len(),
        String::new(),
        "add a stop — optional".to_string(),
    ));
    view! {
        <form
            class=(if revealed { "flight-form form-dock" } else { "flight-form" })
            data-airports-url=(AIRPORTS_JSON)
        >
            <header class=(if revealed { "form-head form-head--dock" } else { "form-head" })>
                <p class="eyebrow">
                    <a href="/thoughts">"thoughts"</a>
                    (if revealed { " · how bad are planes" } else { " / how bad are planes" })
                </p>
            </header>
            <div class="route-fields">
                // DOM (and therefore focus) order is flight order — From,
                // vias, To — so tabbing through a label-hidden docked strip
                // can't land on a field other than the one it appears to.
                // The big strip still leads with From | To: CSS pins
                // .field--to beside From and lets the vias flow beneath.
                airport_field(label: "From".to_string(), name: "from", iata: from.map(|a| a.iata).unwrap_or_default())
                for f in via_fields {
                    airport_field(label: f.label, name: "via", iata: f.iata, hint: f.hint, field_id: f.id, extra_class: "field--via")
                }
                airport_field(label: "To".to_string(), name: "to", iata: to.map(|a| a.iata).unwrap_or_default(), extra_class: "field--to")
            </div>
            <div class="trip-options">
                <label>
                    <input type="radio" name="cabin" value="economy" checked=(cabin == Cabin::Economy)>
                    "Economy"
                </label>
                <label>
                    <input type="radio" name="cabin" value="business" checked=(cabin == Cabin::Business)>
                    "Business"
                </label>
                <label>
                    <input type="radio" name="cabin" value="first" checked=(cabin == Cabin::First)>
                    "First"
                </label>
                <label>
                    <input type="radio" name="trip" value="round" checked=(round_trip)>
                    "Round trip"
                </label>
                <label>
                    <input type="radio" name="trip" value="oneway" checked=(!round_trip)>
                    "One way"
                </label>
            </div>
            if revealed && !total.is_empty() {
                stamp_seal(text: format!("{total} CO₂e"))
            }
            // Unlike the original (where React recomputes live and the button
            // disappears after reveal), the server round-trip always needs a
            // submit control, so the button stays — condensed by .form-dock.
            <button type="submit" class="print-btn">"See how it compares"</button>
        </form>
        <script type="module" src=(AIRPORT_COMBOBOX_JS)></script>
    }
}

/// One route stop. Plain text input for the GET baseline; the client
/// script upgrades `.combobox[data-airport-combobox]` into a typeahead.
/// `hint` overrides the default placeholder; `field_id` overrides the
/// `airport-{name}` input id (the via slots share a name, so each passes
/// its own).
#[component]
async fn airport_field(
    label: String,
    name: &str,
    iata: String,
    #[default(String::new())] hint: String,
    #[default(String::new())] field_id: String,
    #[default("")] extra_class: &str,
) -> Result {
    let input_id = if field_id.is_empty() {
        format!("airport-{name}")
    } else {
        field_id
    };
    let placeholder = if hint.is_empty() {
        format!("{label} — city or code")
    } else {
        hint
    };
    let field_class = if extra_class.is_empty() {
        "field".to_string()
    } else {
        format!("field {extra_class}")
    };
    view! {
        <div class=(field_class.as_str())>
            <label for=(input_id.as_str())>(label)</label>
            <div class="combobox" data-airport-combobox="">
                <input
                    id=(input_id.as_str())
                    type="text"
                    name=(name)
                    role="combobox"
                    placeholder=(placeholder.as_str())
                    autocomplete="off"
                    spellcheck="false"
                    value=(iata.as_str())
                >
            </div>
        </div>
    }
}
