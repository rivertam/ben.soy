//! Flight emissions per the myclimate flight calculator fuel model (2018
//! parameter set, as used by shameplane.com), with the original's blanket
//! ×2 "non-CO₂ effects" multiplier retired in favor of an itemized bill.
//! Per kg of jet fuel, allocated to a passenger:
//!
//!   CO₂, burning the fuel        EF   = 3.15 kg CO₂
//!   Making the fuel (WTT)        P    = 0.51 kg CO₂e
//!   Contrail cirrus              0.63 × CO₂ × sky factor   (GWP100)
//!   NOx + aerosols + water       0.11 × CO₂                (GWP100)
//!
//! The 0.63 and 0.11 come from Lee et al. (2021), Table 5 — 2018
//! CO₂-equivalent emissions at GWP100: contrail cirrus 652 Tg vs CO₂
//! 1034 Tg → 0.63; net NOx 163 + soot 11 + sulfate −84 + water vapour
//! 23 = 113 Tg → 0.11. Summed, CO₂ × 1.74 ≈ the ×1.7 aviation uplift the
//! UK's official 2025 conversion factors adopt from the same table (their
//! uplift, like ours, applies to combustion CO₂ only, not to WTT). The
//! choice of the 100-year clock keeps every number on the page — meals,
//! miles, flights — in the same currency. On a 20-year clock the altitude
//! total runs ≈4× larger (contrails ≈3.7×, NOx & other ≈6.2× — Table 5's
//! GWP20 column); on Lee's numbers the contrail term's
//! 5–95% range spans ≈⅓×–1.7× the central value, and the newest global
//! simulation (Teoh et al. 2024) lands near the bottom of that range.
//!
//! The sky factor re-prices the contrail line by where the route flies.
//! Teoh et al. (2024), Table 2, gives contrail energy forcing per km flown
//! in eleven regions (bounding boxes from their Table S5), attributed to
//! where each contrail formed; dividing by the global mean (0.164 × 10⁸
//! J m⁻¹) makes each a dimensionless intensity — North Atlantic ≈2.4×,
//! Europe ≈1.4×, East Asia ≈0.24×. We sample the great circle and average.
//! Only the *pattern* comes from Teoh; the absolute level stays calibrated
//! to Lee. Weather decides individual flights (≈2.7% of flights cause 80%
//! of contrail forcing) — this line is the route's climatological average.
//!
//! Sea ice: Notz & Stroeve (2016) is defined as 3 ± 0.3 m² of September
//! Arctic sea ice per tonne of CO₂ — the CO₂ line only, so contrails and
//! NOx don't melt receipt ice. (The original site applied it to the ×2
//! total, roughly doubling the ice.)
//!
//! Corrections to the original site kept from earlier revisions: the a·x²
//! term is actually squared (the original's `^` was bitwise XOR),
//! 1500–2500 km interpolates between haul models per the myclimate docs,
//! and the cabin-class weights are un-swapped — the myclimate table gives
//! short-haul economy 0.96 / business 1.26 and long-haul economy 0.80 /
//! business 1.54 (lie-flat business claims more of a wide-body's floor);
//! the original had the two pairs exchanged. One knowing simplification:
//! the 1500/2500 km haul cutoffs are applied to great-circle distance
//! before the detour constant, where the doc defines them on x = GCD + DC
//! — negligible either way.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cabin {
    Economy,
    Business,
    First,
}

impl Cabin {
    pub fn as_str(self) -> &'static str {
        match self {
            Cabin::Economy => "economy",
            Cabin::Business => "business",
            Cabin::First => "first",
        }
    }

    pub fn parse(s: &str) -> Option<Cabin> {
        match s {
            "economy" => Some(Cabin::Economy),
            "business" => Some(Cabin::Business),
            "first" => Some(Cabin::First),
            _ => None,
        }
    }
}

const EARTH_RADIUS_KM: f64 = 6371.009;

const PLF: f64 = 0.77; // passenger load factor
const PASSENGER_SHARE: f64 = 0.951; // 1 − cargo factor
const EF: f64 = 3.15; // kg CO₂ per kg jet fuel, combustion
const P: f64 = 0.51; // fuel pre-production ("well-to-tank"), kg CO₂e per kg fuel

// Lee et al. (2021) Table 5, GWP100, per unit of combustion CO₂
const CONTRAIL_PER_CO2: f64 = 0.63;
const NOX_OTHER_PER_CO2: f64 = 0.11;

const SHORT_HAUL_KM: f64 = 1500.0;
const LONG_HAUL_KM: f64 = 2500.0;

struct CabinWeight {
    economy: f64,
    business: f64,
    first: f64,
}

impl CabinWeight {
    fn get(&self, cabin: Cabin) -> f64 {
        match cabin {
            Cabin::Economy => self.economy,
            Cabin::Business => self.business,
            Cabin::First => self.first,
        }
    }
}

struct HaulModel {
    seats: f64,
    detour_km: f64,
    cabin_weight: CabinWeight,
    a: f64,
    b: f64,
    c: f64,
}

const SHORT_HAUL: HaulModel = HaulModel {
    seats: 158.44,
    detour_km: 50.0,
    cabin_weight: CabinWeight {
        economy: 0.96,
        business: 1.26,
        first: 2.4,
    },
    a: 0.0000387871,
    b: 2.9866,
    c: 1263.42,
};

const LONG_HAUL: HaulModel = HaulModel {
    seats: 280.39,
    detour_km: 125.0,
    cabin_weight: CabinWeight {
        economy: 0.8,
        business: 1.54,
        first: 2.4,
    },
    a: 0.000134576,
    b: 6.1798,
    c: 3446.2,
};

/// The haul model's average seat count, exposed for the seat-map
/// instrument's tests: its drawn cabin must stay honest to the model it
/// depicts.
#[cfg(test)]
pub fn seat_count(long_haul: bool) -> f64 {
    if long_haul {
        LONG_HAUL.seats
    } else {
        SHORT_HAUL.seats
    }
}

/// A cabin's myclimate weight relative to the average seat, same purpose.
pub fn cabin_weight(long_haul: bool, cabin: Cabin) -> f64 {
    if long_haul {
        LONG_HAUL.cabin_weight.get(cabin)
    } else {
        SHORT_HAUL.cabin_weight.get(cabin)
    }
}

pub fn great_circle_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let to_rad = std::f64::consts::PI / 180.0;
    let phi1 = lat1 * to_rad;
    let phi2 = lat2 * to_rad;
    let delta_lambda = (lon2 - lon1) * to_rad;
    let a = (phi2.cos() * delta_lambda.sin()).powi(2)
        + (phi1.cos() * phi2.sin() - phi1.sin() * phi2.cos() * delta_lambda.cos()).powi(2);
    let b = phi1.sin() * phi2.sin() + phi1.cos() * phi2.cos() * delta_lambda.cos();
    a.sqrt().atan2(b) * EARTH_RADIUS_KM
}

/// Total jet fuel for one leg, kg — the myclimate fuel curve, whole aircraft.
fn haul_fuel_curve(model: &HaulModel, distance_km: f64) -> f64 {
    let x = distance_km + model.detour_km;
    model.a * x * x + model.b * x + model.c
}

/// Jet fuel per passenger for one leg, kg — the myclimate fuel curve, allocated.
fn haul_fuel_kg(model: &HaulModel, distance_km: f64, cabin: Cabin) -> f64 {
    (haul_fuel_curve(model, distance_km) / (model.seats * PLF))
        * PASSENGER_SHARE
        * model.cabin_weight.get(cabin)
}

fn aircraft_fuel_kg_per_leg(distance_km: f64) -> f64 {
    if distance_km <= SHORT_HAUL_KM {
        return haul_fuel_curve(&SHORT_HAUL, distance_km);
    }
    if distance_km >= LONG_HAUL_KM {
        return haul_fuel_curve(&LONG_HAUL, distance_km);
    }
    let t = (distance_km - SHORT_HAUL_KM) / (LONG_HAUL_KM - SHORT_HAUL_KM);
    (1.0 - t) * haul_fuel_curve(&SHORT_HAUL, SHORT_HAUL_KM)
        + t * haul_fuel_curve(&LONG_HAUL, LONG_HAUL_KM)
}

pub fn per_leg_fuel_kg(distance_km: f64, cabin: Cabin) -> f64 {
    if distance_km <= SHORT_HAUL_KM {
        return haul_fuel_kg(&SHORT_HAUL, distance_km, cabin);
    }
    if distance_km >= LONG_HAUL_KM {
        return haul_fuel_kg(&LONG_HAUL, distance_km, cabin);
    }
    let t = (distance_km - SHORT_HAUL_KM) / (LONG_HAUL_KM - SHORT_HAUL_KM);
    (1.0 - t) * haul_fuel_kg(&SHORT_HAUL, SHORT_HAUL_KM, cabin)
        + t * haul_fuel_kg(&LONG_HAUL, LONG_HAUL_KM, cabin)
}

/// Contrail intensity per km flown, by region: Teoh et al. (2024) Table 2,
/// "EFcontrail per flight distance" (×10⁸ J m⁻¹, attributed to the formation
/// location), with bounding boxes (west, south, east, north) from Table S5.
/// The paper's boxes overlap; listing order resolves ties, most specific
/// airspace first. Everywhere unlisted bills at the global mean.
const GLOBAL_EF_PER_KM: f64 = 0.164;

struct ContrailRegion {
    #[allow(dead_code)]
    name: &'static str,
    west: f64,
    south: f64,
    east: f64,
    north: f64,
    ef_per_km: f64,
}

#[rustfmt::skip]
const CONTRAIL_REGIONS: [ContrailRegion; 11] = [
    ContrailRegion { name: "Arctic", west: -180.0, south: 66.5, east: 180.0, north: 90.0, ef_per_km: 0.267 },
    ContrailRegion { name: "Europe", west: -12.0, south: 35.0, east: 20.0, north: 60.0, ef_per_km: 0.227 },
    ContrailRegion { name: "China", west: 73.5, south: 18.0, east: 135.0, north: 53.5, ef_per_km: 0.046 },
    ContrailRegion { name: "India", west: 68.0, south: 8.0, east: 97.5, north: 35.5, ef_per_km: 0.052 },
    ContrailRegion { name: "East Asia", west: 103.0, south: 15.0, east: 150.0, north: 48.0, ef_per_km: 0.04 },
    ContrailRegion { name: "Southeast Asia", west: 87.5, south: -10.0, east: 130.0, north: 20.0, ef_per_km: 0.112 },
    ContrailRegion { name: "USA", west: -126.0, south: 23.0, east: -66.0, north: 50.0, ef_per_km: 0.134 },
    ContrailRegion { name: "North Atlantic", west: -70.0, south: 40.0, east: -5.0, north: 63.0, ef_per_km: 0.39 },
    ContrailRegion { name: "North Pacific", west: 140.0, south: 35.0, east: -120.0, north: 65.0, ef_per_km: 0.146 },
    ContrailRegion { name: "Latin America", west: -85.0, south: -60.0, east: -35.0, north: 15.0, ef_per_km: 0.105 },
    ContrailRegion { name: "Africa & Middle East", west: -20.0, south: -35.0, east: 50.0, north: 40.0, ef_per_km: 0.082 },
];

fn point_sky_factor(lat_deg: f64, lon_deg: f64) -> f64 {
    for r in &CONTRAIL_REGIONS {
        if lat_deg < r.south || lat_deg > r.north {
            continue;
        }
        // A box with west > east (the North Pacific) wraps the antimeridian.
        let in_lon = if r.west <= r.east {
            lon_deg >= r.west && lon_deg <= r.east
        } else {
            lon_deg >= r.west || lon_deg <= r.east
        };
        if in_lon {
            return r.ef_per_km / GLOBAL_EF_PER_KM;
        }
    }
    1.0
}

const SKY_SAMPLE_KM: f64 = 200.0;

/// Mean contrail intensity along the route's great circle, relative to the
/// global average km flown: 1.0 is a typical sky, the North Atlantic corridor
/// ≈2.4, subtropical East Asia ≈0.24.
pub fn contrail_sky_factor(from: Coordinates, to: Coordinates) -> f64 {
    let to_rad = std::f64::consts::PI / 180.0;
    let to_deg = 180.0 / std::f64::consts::PI;
    let distance_km = great_circle_km(from.lat, from.lon, to.lat, to.lon);
    if distance_km < 1.0 {
        return point_sky_factor(from.lat, from.lon);
    }

    let ax = (from.lat * to_rad).cos() * (from.lon * to_rad).cos();
    let ay = (from.lat * to_rad).cos() * (from.lon * to_rad).sin();
    let az = (from.lat * to_rad).sin();
    let bx = (to.lat * to_rad).cos() * (to.lon * to_rad).cos();
    let by = (to.lat * to_rad).cos() * (to.lon * to_rad).sin();
    let bz = (to.lat * to_rad).sin();

    let omega = (ax * bx + ay * by + az * bz).clamp(-1.0, 1.0).acos();
    let sin_omega = omega.sin();
    // Antipodal endpoints leave the great circle undefined; bill the global mean.
    if sin_omega < 1e-9 {
        return 1.0;
    }

    let steps = ((distance_km / SKY_SAMPLE_KM).ceil() as i64).max(1);
    let mut sum = 0.0;
    for i in 0..=steps {
        let t = i as f64 / steps as f64;
        let s1 = ((1.0 - t) * omega).sin() / sin_omega;
        let s2 = (t * omega).sin() / sin_omega;
        let x = s1 * ax + s2 * bx;
        let y = s1 * ay + s2 * by;
        let z = s1 * az + s2 * bz;
        sum += point_sky_factor(z.atan2(x.hypot(y)) * to_deg, y.atan2(x) * to_deg);
    }
    sum / (steps + 1) as f64
}

/// ≈3 m² of September Arctic sea ice lost per tonne of CO₂ (Notz & Stroeve, 2016).
pub const ICE_M2_PER_TONNE: f64 = 3.0;

/// Jet A-1 standard density at 15 °C, kg per litre (Measurement Canada VCF tables).
pub const JET_FUEL_KG_PER_LITRE: f64 = 0.8;

/// Paris-aligned personal allowance for all mobility (car, bus, train,
/// plane), tonnes CO₂e per person per year: the 2030 milestone of the
/// 1.5-Degree Lifestyles technical report (IGES/Aalto/D-mat 2019) —
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Coordinates {
    pub lat: f64,
    pub lon: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct FlightInput {
    pub from: Coordinates,
    pub to: Coordinates,
    pub cabin: Cabin,
    pub round_trip: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct FlightImpact {
    pub distance_km: f64,
    /// The whole bill, tonnes CO₂e at GWP100.
    pub tonnes_co2e: f64,
    /// Combustion CO₂ alone — the line the sea ice answers to.
    pub co2_tonnes: f64,
    /// Jet fuel burned, this ticket's share.
    pub fuel_kg: f64,
    /// Fuel pre-production (well-to-tank).
    pub wtt_tonnes: f64,
    /// Contrail cirrus, expected value for this route's skies (GWP100).
    pub contrail_tonnes: f64,
    /// NOx, aerosols and water vapour, net (GWP100).
    pub nox_other_tonnes: f64,
    /// This route's contrail intensity vs. the global average km flown.
    pub sky_factor: f64,
    /// This ticket's share of the whole aircraft's fuel burn (and thus every line).
    pub seat_share_of_aircraft: f64,
    /// Whole-aircraft bill for the same itinerary, tonnes CO₂e at GWP100.
    pub aircraft_tonnes_co2e: f64,
    pub ice_m2: f64,
}

/// The all-zeros bill: `distance_km` is an exact 0.0 because the receipt's
/// staycation branch compares it with `==`.
fn zero_impact() -> FlightImpact {
    FlightImpact {
        distance_km: 0.0,
        tonnes_co2e: 0.0,
        co2_tonnes: 0.0,
        fuel_kg: 0.0,
        wtt_tonnes: 0.0,
        contrail_tonnes: 0.0,
        nox_other_tonnes: 0.0,
        sky_factor: 1.0,
        seat_share_of_aircraft: 1.0,
        aircraft_tonnes_co2e: 0.0,
        ice_m2: 0.0,
    }
}

pub fn flight_impact(input: &FlightInput) -> FlightImpact {
    let &FlightInput {
        from,
        to,
        cabin,
        round_trip,
    } = input;
    let distance_km = great_circle_km(from.lat, from.lon, to.lat, to.lon);
    if distance_km < 0.5 {
        return zero_impact();
    }
    let legs = if round_trip { 2.0 } else { 1.0 };
    let seat_share_of_aircraft =
        per_leg_fuel_kg(distance_km, cabin) / aircraft_fuel_kg_per_leg(distance_km);
    let fuel_kg = per_leg_fuel_kg(distance_km, cabin) * legs;
    let sky_factor = contrail_sky_factor(from, to);

    let co2_tonnes = (fuel_kg * EF) / 1000.0;
    let wtt_tonnes = (fuel_kg * P) / 1000.0;
    let contrail_tonnes = co2_tonnes * CONTRAIL_PER_CO2 * sky_factor;
    let nox_other_tonnes = co2_tonnes * NOX_OTHER_PER_CO2;
    let tonnes_co2e = co2_tonnes + wtt_tonnes + contrail_tonnes + nox_other_tonnes;

    FlightImpact {
        distance_km,
        tonnes_co2e,
        co2_tonnes,
        fuel_kg,
        wtt_tonnes,
        contrail_tonnes,
        nox_other_tonnes,
        sky_factor,
        seat_share_of_aircraft,
        aircraft_tonnes_co2e: tonnes_co2e / seat_share_of_aircraft,
        ice_m2: co2_tonnes * ICE_M2_PER_TONNE,
    }
}

/// A multi-leg itinerary: `stops` flown in order (and all legs flown back
/// again when `round_trip`). Each leg is priced independently — the fuel
/// curve's constant term, per-leg detour constant, haul-model choice, and
/// contrail sky factor are all per-leg quantities, which is exactly why a
/// layover bills differently than the nonstop. Sub-half-km legs (adjacent
/// duplicate stops) contribute nothing, and an itinerary with no real leg
/// returns the same exact-zero bill as a staycation.
///
/// Aggregates keep `flight_impact`'s invariants: the four lines sum to
/// `tonnes_co2e`; `sky_factor` becomes the CO₂-weighted mean of leg skies
/// (so `contrail = co2 × 0.63 × sky` still holds); `aircraft_tonnes_co2e`
/// sums the legs' whole-aircraft bills and `seat_share_of_aircraft` is
/// derived from it, preserving `aircraft == tonnes / seat_share`.
/// `distance_km` stays the one-way sum, mirroring the single-leg convention.
pub fn route_impact(stops: &[Coordinates], cabin: Cabin, round_trip: bool) -> FlightImpact {
    let legs: Vec<FlightImpact> = stops
        .windows(2)
        .map(|w| {
            flight_impact(&FlightInput {
                from: w[0],
                to: w[1],
                cabin,
                round_trip: false,
            })
        })
        .filter(|leg| leg.distance_km > 0.0)
        .collect();
    if legs.is_empty() {
        return zero_impact();
    }

    let trip_factor = if round_trip { 2.0 } else { 1.0 };
    let distance_km: f64 = legs.iter().map(|l| l.distance_km).sum();
    let fuel_kg: f64 = legs.iter().map(|l| l.fuel_kg).sum::<f64>() * trip_factor;
    let contrail_tonnes: f64 = legs.iter().map(|l| l.contrail_tonnes).sum::<f64>() * trip_factor;
    let aircraft_tonnes_co2e: f64 =
        legs.iter().map(|l| l.aircraft_tonnes_co2e).sum::<f64>() * trip_factor;

    let co2_tonnes = (fuel_kg * EF) / 1000.0;
    let wtt_tonnes = (fuel_kg * P) / 1000.0;
    let nox_other_tonnes = co2_tonnes * NOX_OTHER_PER_CO2;
    let tonnes_co2e = co2_tonnes + wtt_tonnes + contrail_tonnes + nox_other_tonnes;

    FlightImpact {
        distance_km,
        tonnes_co2e,
        co2_tonnes,
        fuel_kg,
        wtt_tonnes,
        contrail_tonnes,
        nox_other_tonnes,
        // A lone leg keeps its factor verbatim: the CO₂-weighted quotient
        // re-derives the same number but perturbs it by an ulp, and the
        // dataset parks routes exactly on the receipt sky-note's ×0.75 and
        // ×1.25 sentence boundaries.
        sky_factor: if legs.len() == 1 {
            legs[0].sky_factor
        } else {
            contrail_tonnes / (co2_tonnes * CONTRAIL_PER_CO2)
        },
        seat_share_of_aircraft: tonnes_co2e / aircraft_tonnes_co2e,
        aircraft_tonnes_co2e,
        ice_m2: co2_tonnes * ICE_M2_PER_TONNE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const JFK: Coordinates = Coordinates {
        lat: 40.6413,
        lon: -73.7781,
    };
    const LHR: Coordinates = Coordinates {
        lat: 51.47,
        lon: -0.4543,
    };

    #[test]
    fn great_circle_jfk_lhr_is_about_5540_km() {
        let d = great_circle_km(JFK.lat, JFK.lon, LHR.lat, LHR.lon);
        assert!((d - 5540.0).abs() < 60.0, "got {d}");
        let back = great_circle_km(LHR.lat, LHR.lon, JFK.lat, JFK.lon);
        assert!((d - back).abs() < 1e-9);
    }

    #[test]
    fn fuel_curve_is_continuous_across_haul_boundaries() {
        for boundary in [SHORT_HAUL_KM, LONG_HAUL_KM] {
            let below = per_leg_fuel_kg(boundary - 1e-6, Cabin::Economy);
            let above = per_leg_fuel_kg(boundary + 1e-6, Cabin::Economy);
            assert!(
                (below - above).abs() < 0.1,
                "discontinuity at {boundary}: {below} vs {above}"
            );
        }
    }

    #[test]
    fn staycation_distance_is_exactly_zero() {
        // The receipt's staycation branch compares distance_km == 0.0; the
        // sub-half-km short circuit must keep returning an exact 0.0.
        let impact = flight_impact(&FlightInput {
            from: JFK,
            to: JFK,
            cabin: Cabin::Economy,
            round_trip: false,
        });
        assert_eq!(impact.distance_km, 0.0);
        assert_eq!(impact.tonnes_co2e, 0.0);
        assert_eq!(impact.seat_share_of_aircraft, 1.0);
    }

    #[test]
    fn round_trip_doubles_fuel() {
        let one = flight_impact(&FlightInput {
            from: JFK,
            to: LHR,
            cabin: Cabin::Economy,
            round_trip: false,
        });
        let two = flight_impact(&FlightInput {
            from: JFK,
            to: LHR,
            cabin: Cabin::Economy,
            round_trip: true,
        });
        assert!((two.fuel_kg - 2.0 * one.fuel_kg).abs() < 1e-9);
        assert!((two.tonnes_co2e - 2.0 * one.tonnes_co2e).abs() < 1e-9);
    }

    #[test]
    fn impact_lines_sum_to_the_total() {
        let i = flight_impact(&FlightInput {
            from: JFK,
            to: LHR,
            cabin: Cabin::Business,
            round_trip: false,
        });
        let sum = i.co2_tonnes + i.wtt_tonnes + i.contrail_tonnes + i.nox_other_tonnes;
        assert!((i.tonnes_co2e - sum).abs() < 1e-12);
        assert!((i.ice_m2 - i.co2_tonnes * ICE_M2_PER_TONNE).abs() < 1e-12);
        assert!((i.aircraft_tonnes_co2e - i.tonnes_co2e / i.seat_share_of_aircraft).abs() < 1e-12);
    }

    #[test]
    fn sky_factor_regions() {
        // Mid North Atlantic: the corridor premium.
        assert!((point_sky_factor(50.0, -30.0) - 0.39 / GLOBAL_EF_PER_KM).abs() < 1e-12);
        // North Pacific box wraps the antimeridian: both sides of ±180 hit it.
        assert!((point_sky_factor(45.0, 175.0) - 0.146 / GLOBAL_EF_PER_KM).abs() < 1e-12);
        assert!((point_sky_factor(55.0, -150.0) - 0.146 / GLOBAL_EF_PER_KM).abs() < 1e-12);
        // Open ocean south of every box: global mean.
        assert!((point_sky_factor(-50.0, -150.0) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn antipodal_route_bills_the_global_mean() {
        let a = Coordinates { lat: 0.0, lon: 0.0 };
        let b = Coordinates {
            lat: 0.0,
            lon: 180.0,
        };
        assert_eq!(contrail_sky_factor(a, b), 1.0);
    }

    #[test]
    fn jfk_lhr_flies_expensive_skies() {
        // The route crosses the North Atlantic corridor; its average must
        // land above the global mean but below the corridor peak.
        let f = contrail_sky_factor(JFK, LHR);
        assert!(f > 1.2 && f < 2.4, "got {f}");
    }

    const KEF: Coordinates = Coordinates {
        lat: 63.985,
        lon: -22.6056,
    };

    fn assert_close(a: f64, b: f64, what: &str) {
        let scale = a.abs().max(b.abs()).max(1e-12);
        assert!((a - b).abs() / scale < 1e-12, "{what}: {a} vs {b}");
    }

    #[test]
    fn nonstop_route_matches_flight_impact() {
        let nonstop = flight_impact(&FlightInput {
            from: JFK,
            to: LHR,
            cabin: Cabin::Business,
            round_trip: true,
        });
        let route = route_impact(&[JFK, LHR], Cabin::Business, true);
        assert_close(route.distance_km, nonstop.distance_km, "distance");
        assert_close(route.fuel_kg, nonstop.fuel_kg, "fuel");
        assert_close(route.tonnes_co2e, nonstop.tonnes_co2e, "total");
        assert_close(route.contrail_tonnes, nonstop.contrail_tonnes, "contrail");
        // Bit-exact, not merely close: real routes land exactly on the
        // receipt sky-note's ×0.75/×1.25 sentence boundaries, so an ulp of
        // drift flips its wording on old share URLs.
        assert_eq!(route.sky_factor.to_bits(), nonstop.sky_factor.to_bits());
        assert_close(
            route.seat_share_of_aircraft,
            nonstop.seat_share_of_aircraft,
            "seat share",
        );
        assert_close(
            route.aircraft_tonnes_co2e,
            nonstop.aircraft_tonnes_co2e,
            "aircraft",
        );
    }

    #[test]
    fn layover_burns_more_fuel_than_nonstop() {
        // Each leg pays its own fuel-curve intercept and detour constant, so
        // connecting through Keflavík must out-burn the JFK–LHR nonstop.
        let nonstop = flight_impact(&FlightInput {
            from: JFK,
            to: LHR,
            cabin: Cabin::Economy,
            round_trip: false,
        });
        let via_kef = route_impact(&[JFK, KEF, LHR], Cabin::Economy, false);
        assert!(via_kef.distance_km > nonstop.distance_km);
        assert!(
            via_kef.fuel_kg > nonstop.fuel_kg,
            "{} vs {}",
            via_kef.fuel_kg,
            nonstop.fuel_kg
        );
    }

    #[test]
    fn route_sums_its_legs_and_keeps_the_invariants() {
        let leg = |from, to| {
            flight_impact(&FlightInput {
                from,
                to,
                cabin: Cabin::Economy,
                round_trip: false,
            })
        };
        let (a, b) = (leg(JFK, KEF), leg(KEF, LHR));
        let route = route_impact(&[JFK, KEF, LHR], Cabin::Economy, false);
        assert_close(route.distance_km, a.distance_km + b.distance_km, "distance");
        assert_close(route.fuel_kg, a.fuel_kg + b.fuel_kg, "fuel");
        assert_close(
            route.contrail_tonnes,
            a.contrail_tonnes + b.contrail_tonnes,
            "contrail",
        );
        assert_close(
            route.aircraft_tonnes_co2e,
            a.aircraft_tonnes_co2e + b.aircraft_tonnes_co2e,
            "aircraft",
        );
        // The two legs fly different skies; the aggregate factor must sit
        // between them, priced so the contrail identity still holds.
        let (lo, hi) = (
            a.sky_factor.min(b.sky_factor),
            a.sky_factor.max(b.sky_factor),
        );
        assert!(route.sky_factor > lo && route.sky_factor < hi);
        let sum =
            route.co2_tonnes + route.wtt_tonnes + route.contrail_tonnes + route.nox_other_tonnes;
        assert!((route.tonnes_co2e - sum).abs() < 1e-12);
        assert_close(
            route.contrail_tonnes,
            route.co2_tonnes * CONTRAIL_PER_CO2 * route.sky_factor,
            "contrail identity",
        );
        assert_close(
            route.aircraft_tonnes_co2e,
            route.tonnes_co2e / route.seat_share_of_aircraft,
            "aircraft identity",
        );
        assert_close(route.ice_m2, route.co2_tonnes * ICE_M2_PER_TONNE, "ice");
    }

    #[test]
    fn route_round_trip_doubles_fuel_but_not_distance() {
        let one = route_impact(&[JFK, KEF, LHR], Cabin::Economy, false);
        let two = route_impact(&[JFK, KEF, LHR], Cabin::Economy, true);
        assert_close(two.fuel_kg, 2.0 * one.fuel_kg, "fuel");
        assert_close(two.tonnes_co2e, 2.0 * one.tonnes_co2e, "total");
        assert_close(two.distance_km, one.distance_km, "distance stays one-way");
    }

    #[test]
    fn degenerate_legs_contribute_nothing() {
        let plain = route_impact(&[JFK, LHR], Cabin::Economy, false);
        let stutter = route_impact(&[JFK, JFK, LHR], Cabin::Economy, false);
        assert_close(stutter.tonnes_co2e, plain.tonnes_co2e, "total");
        assert_close(stutter.distance_km, plain.distance_km, "distance");

        // An itinerary with no real leg is still an exact-zero staycation.
        let staycation = route_impact(&[JFK, JFK], Cabin::Economy, false);
        assert_eq!(staycation.distance_km, 0.0);
        assert_eq!(staycation.tonnes_co2e, 0.0);
        assert_eq!(staycation.seat_share_of_aircraft, 1.0);
    }

    #[test]
    fn out_and_back_one_way_equals_the_round_trip() {
        // JFK → LHR → JFK one-way flies the same two legs as the JFK ⇄ LHR
        // round trip; the bills must agree.
        let out_and_back = route_impact(&[JFK, LHR, JFK], Cabin::Economy, false);
        let round = route_impact(&[JFK, LHR], Cabin::Economy, true);
        assert_close(out_and_back.fuel_kg, round.fuel_kg, "fuel");
        assert_close(out_and_back.tonnes_co2e, round.tonnes_co2e, "total");
    }
}
