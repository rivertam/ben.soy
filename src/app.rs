mod admin;
mod diary;
mod diary_sync;
mod favicon;
mod feed;
mod home;
mod interests;
mod llms;
mod log;
pub(crate) mod login;
mod motorcycles;
mod not_found;
mod pwa;
mod response_layer;
mod resume;
pub(crate) mod thoughts;
mod workout_upload;

use benjisponge::data::Data;

use self::interests::lifting::archive::store::FitnessStore;
use topcoat::{
    asset::{AssetBundle, RouterBuilderAssetExt},
    cookie::{Key, RouterBuilderCookieExt},
    router::{Router, RouterBuilderDiscoverExt},
};

pub fn router() -> Router {
    let data = Data::from_env();
    Router::builder()
        .assets(AssetBundle::load().unwrap())
        .discover()
        .cookies()
        .app_context(data.clone())
        .app_context(FitnessStore::new(data))
        .app_context(cookie_key())
        .build()
}

/// The key behind `private_cookies` (the login module's viewer cookie).
/// `COOKIE_KEY` is any secret string of 32+ bytes; without it, a fresh key
/// per boot means viewer sessions silently reset on every restart — fine to
/// ignore until sign-in matters, wrong to ship once it does.
fn cookie_key() -> Key {
    match std::env::var("COOKIE_KEY") {
        Ok(master) if master.len() >= 32 => Key::derive_from(master.as_bytes()),
        Ok(_) => panic!("COOKIE_KEY must be at least 32 bytes"),
        Err(_) => {
            eprintln!(
                "{}",
                serde_json::json!({
                    "message": "COOKIE_KEY unset; viewer sessions will not survive restarts",
                })
            );
            Key::generate()
        }
    }
}
