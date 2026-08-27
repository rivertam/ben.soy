//! Admin-only browser endpoint for publishing a pasted workout.
//!
//! The dialog lives on the public `/fitness` page but is rendered only for the
//! signed-in `ADMIN_EMAIL`. This POST route independently repeats that exact
//! identity check and requires positive same-origin browser evidence before
//! reading a bounded form body.

use std::time::{SystemTime, UNIX_EPOCH};

use benjisponge::data::Data;
use topcoat::{
    Result,
    context::{Cx, app_context},
    router::{
        Body, HeaderMap, HeaderValue, StatusCode, header, request::headers, response::Response,
        route, to_bytes,
    },
};

use crate::content::access::is_admin;

use super::interests::lifting::archive::{
    db::{self, ManualImportOutcome},
    eastern, manual,
    store::FitnessStore,
};
use super::interests::lifting::canonical_share_text;
use super::login::viewer;
use crate::util::is_same_origin;

const PATH: &str = "/fitness/lift/import";
const LOGIN_REDIRECT: &str = "/login?next=%2Ffitness";
// URL encoding can expand each byte to `%HH`; the decoded Lyfta parser keeps
// the authoritative 64 KiB workout-text bound.
const BODY_LIMIT_BYTES: usize = manual::LYFTA_TEXT_LIMIT * 3 + 1_024;
const NO_STORE: &str = "no-store";

#[route(POST "/fitness/lift/import")]
async fn publish_pasted_workout(cx: &Cx, body: Body) -> Result<Response> {
    publish_pasted_workout_inner(cx, body, false).await
}

/// Existing forms, bookmarks, and older bundled upload JavaScript keep their
/// write contract. Its JSON response retains the old `/lifting/...` alias so
/// a cached client accepts the result; following that URL redirects to the
/// canonical Fitness permalink.
#[route(POST "/lifting/upload")]
async fn legacy_publish_pasted_workout(cx: &Cx, body: Body) -> Result<Response> {
    publish_pasted_workout_inner(cx, body, true).await
}

async fn publish_pasted_workout_inner(
    cx: &Cx,
    body: Body,
    legacy_json_location: bool,
) -> Result<Response> {
    let json = wants_json(headers(cx));
    let Some(current) = viewer(cx) else {
        return Ok(if json {
            upload_error(true, StatusCode::UNAUTHORIZED, "Sign in again, then retry.")
        } else {
            see_other(LOGIN_REDIRECT)
        });
    };
    if !is_admin(&current.email) {
        return Ok(upload_error(json, StatusCode::NOT_FOUND, "not found"));
    }
    if !is_same_origin(headers(cx)) {
        return Ok(upload_error(json, StatusCode::FORBIDDEN, "forbidden"));
    }
    if !is_form_content_type(headers(cx)) {
        return Ok(upload_error(
            json,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Content-Type must be application/x-www-form-urlencoded",
        ));
    }
    match declared_body_length(headers(cx)) {
        Ok(Some(length)) if length > BODY_LIMIT_BYTES => {
            return Ok(upload_error(
                json,
                StatusCode::PAYLOAD_TOO_LARGE,
                "form is too large",
            ));
        }
        Ok(_) => {}
        Err(()) => {
            return Ok(upload_error(
                json,
                StatusCode::BAD_REQUEST,
                "bad Content-Length",
            ));
        }
    }

    let bytes = match to_bytes(body, BODY_LIMIT_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return Ok(upload_error(
                json,
                StatusCode::PAYLOAD_TOO_LARGE,
                "form is too large",
            ));
        }
    };
    let workout = match parse_upload_form(&bytes) {
        Ok(workout) => workout,
        Err(_) => return Ok(upload_error(json, StatusCode::BAD_REQUEST, "bad form")),
    };

    let parsed = match manual::parse_lyfta(&workout) {
        Ok(parsed) => parsed,
        Err(error) => {
            return Ok(upload_error(
                json,
                StatusCode::UNPROCESSABLE_ENTITY,
                &format!("Could not parse the Lyfta workout: {error}\n"),
            ));
        }
    };
    let imported_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0);
    let handle = match app_context::<Data>(cx).db().await {
        Ok(handle) => handle,
        Err(error) => {
            log_failure(error);
            return Ok(upload_error(
                json,
                StatusCode::SERVICE_UNAVAILABLE,
                "The workout archive is temporarily unavailable.\n",
            ));
        }
    };
    let outcome = match db::create_manual_workout(&handle, &parsed.payload, imported_at).await {
        Ok(outcome) => outcome,
        Err(error) => {
            log_failure(error);
            return Ok(upload_error(
                json,
                StatusCode::SERVICE_UNAVAILABLE,
                "The workout could not be published right now.\n",
            ));
        }
    };
    let store = app_context::<FitnessStore>(cx);
    match outcome {
        ManualImportOutcome::Added => {
            if let Err(error) = store.rebuild().await {
                // The transaction committed. A stale snapshot is preferable
                // to claiming the upload failed and inviting a retry; the
                // next read's version check will retry the rebuild.
                log_failure(error);
            }
        }
        ManualImportOutcome::Duplicate => {
            // Refresh on an idempotent retry too: another process may have
            // performed the original insert while this process held a stale
            // snapshot. Both the JSON share-text response and the ordinary
            // form's redirect must see the newly stored workout immediately.
            if let Err(error) = store.rebuild().await {
                log_failure(error);
            }
        }
        ManualImportOutcome::Conflict => {
            return Ok(upload_error(
                json,
                StatusCode::CONFLICT,
                "A different workout already uses that start time.\n",
            ));
        }
    }
    let location = published_location(&parsed.public_path, legacy_json_location && json);
    if !json {
        return Ok(see_other(&location));
    }

    let share_text = match stored_share_text(cx, &parsed.public_path).await {
        Ok(text) => Some(text),
        Err(error) => {
            // Publishing already committed. Return the permanent location so
            // the browser never retries a successful write just because the
            // post-commit snapshot was temporarily unavailable.
            log_share_failure(error);
            None
        }
    };
    Ok(json_response(
        StatusCode::OK,
        serde_json::json!({
            "location": location,
            "share_text": share_text,
        }),
    ))
}

fn published_location(public_path: &str, legacy_json_location: bool) -> String {
    if legacy_json_location {
        format!("/lifting/{public_path}")
    } else {
        format!("/fitness/lift/{public_path}")
    }
}

async fn stored_share_text(cx: &Cx, public_path: &str) -> std::result::Result<String, String> {
    let instant = eastern::parse_public_path(public_path)
        .ok_or_else(|| "generated workout path did not parse".to_string())?;
    let snapshot = app_context::<FitnessStore>(cx)
        .snapshot()
        .await
        .map_err(|error| error.to_string())?;
    let workout = snapshot
        .by_path(&instant)
        .and_then(|detail| detail.workout)
        .ok_or_else(|| "published workout was absent from the fresh snapshot".to_string())?;
    Ok(canonical_share_text(cx, &workout))
}

fn wants_json(headers: &HeaderMap) -> bool {
    headers
        .get_all(header::ACCEPT)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|media_range| media_range.split(';').next())
        .any(|media_type| media_type.trim().eq_ignore_ascii_case("application/json"))
}

fn is_form_content_type(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| {
            value
                .trim()
                .eq_ignore_ascii_case("application/x-www-form-urlencoded")
        })
}

/// Returns a single trustworthy Content-Length, or `None` for a chunked body.
/// Duplicate, non-UTF-8, and non-integer declarations are rejected.
fn declared_body_length(headers: &HeaderMap) -> std::result::Result<Option<usize>, ()> {
    let mut values = headers.get_all(header::CONTENT_LENGTH).iter();
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(());
    }
    value
        .to_str()
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .map(Some)
        .ok_or(())
}

#[derive(Debug, Eq, PartialEq)]
enum UploadFormError {
    BadEncoding,
    DuplicateWorkout,
    EmptyWorkout,
    MissingWorkout,
    UnexpectedField,
}

/// Strictly decode exactly one `workout` URL-encoded field. Browser-generated
/// forms always use valid percent escapes and UTF-8; rejecting malformed input
/// keeps ambiguous alternate decodings out of the future parser.
fn parse_upload_form(body: &[u8]) -> std::result::Result<String, UploadFormError> {
    if body.is_empty() {
        return Err(UploadFormError::MissingWorkout);
    }
    let mut workout = None;
    for pair in body.split(|byte| *byte == b'&') {
        if pair.is_empty() {
            return Err(UploadFormError::UnexpectedField);
        }
        let separator = pair.iter().position(|byte| *byte == b'=');
        let (key, value) = match separator {
            Some(index) => (&pair[..index], &pair[index + 1..]),
            None => (pair, &[][..]),
        };
        let key = decode_form_component(key)?;
        if key != "workout" {
            return Err(UploadFormError::UnexpectedField);
        }
        let value = decode_form_component(value)?;
        if workout.replace(value).is_some() {
            return Err(UploadFormError::DuplicateWorkout);
        }
    }
    let workout = workout.ok_or(UploadFormError::MissingWorkout)?;
    if workout.trim().is_empty() {
        return Err(UploadFormError::EmptyWorkout);
    }
    Ok(workout)
}

fn decode_form_component(encoded: &[u8]) -> std::result::Result<String, UploadFormError> {
    let mut decoded = Vec::with_capacity(encoded.len());
    let mut index = 0;
    while index < encoded.len() {
        match encoded[index] {
            b'+' => decoded.push(b' '),
            b'%' => {
                let Some(high) = encoded.get(index + 1).and_then(|byte| hex_value(*byte)) else {
                    return Err(UploadFormError::BadEncoding);
                };
                let Some(low) = encoded.get(index + 2).and_then(|byte| hex_value(*byte)) else {
                    return Err(UploadFormError::BadEncoding);
                };
                decoded.push((high << 4) | low);
                index += 2;
            }
            byte => decoded.push(byte),
        }
        index += 1;
    }
    String::from_utf8(decoded).map_err(|_| UploadFormError::BadEncoding)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn see_other(location: &str) -> Response {
    let mut response = text_response(StatusCode::SEE_OTHER, "see other");
    let location = HeaderValue::from_str(location).expect("workout redirect is a valid path");
    response.headers_mut().insert(header::LOCATION, location);
    response
}

fn text_response(status: StatusCode, message: &str) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::CACHE_CONTROL, NO_STORE)
        .header("x-content-type-options", "nosniff")
        .header(header::REFERRER_POLICY, "no-referrer")
        .body(Body::from(message.to_string()))
        .expect("upload response uses static headers")
}

fn upload_error(json: bool, status: StatusCode, message: &str) -> Response {
    if json {
        json_response(
            status,
            serde_json::json!({
                "error": message.trim(),
            }),
        )
    } else {
        text_response(status, message)
    }
}

fn json_response(status: StatusCode, value: serde_json::Value) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .header(header::CACHE_CONTROL, NO_STORE)
        .header("x-content-type-options", "nosniff")
        .header(header::REFERRER_POLICY, "no-referrer")
        .body(Body::from(value.to_string()))
        .expect("upload JSON response uses static headers")
}

fn log_failure(error: impl std::fmt::Display) {
    eprintln!(
        "{}",
        serde_json::json!({
            "message": "manual workout publish failed",
            "path": PATH,
            "error": error.to_string(),
        })
    );
}

fn log_share_failure(error: impl std::fmt::Display) {
    eprintln!(
        "{}",
        serde_json::json!({
            "message": "manual workout published but share text was unavailable",
            "path": PATH,
            "error": error.to_string(),
        })
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_type_accepts_a_urlencoded_form_with_parameters() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("Application/X-Www-Form-Urlencoded; charset=UTF-8"),
        );
        assert!(is_form_content_type(&headers));

        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"));
        assert!(!is_form_content_type(&headers));
        headers.remove(header::CONTENT_TYPE);
        assert!(!is_form_content_type(&headers));
    }

    #[test]
    fn content_length_is_optional_but_must_be_single_and_numeric() {
        let mut headers = HeaderMap::new();
        assert_eq!(declared_body_length(&headers), Ok(None));

        headers.insert(
            header::CONTENT_LENGTH,
            HeaderValue::from_str(&BODY_LIMIT_BYTES.to_string()).unwrap(),
        );
        assert_eq!(declared_body_length(&headers), Ok(Some(BODY_LIMIT_BYTES)));

        headers.insert(header::CONTENT_LENGTH, HeaderValue::from_static("-1"));
        assert_eq!(declared_body_length(&headers), Err(()));

        headers.insert(
            header::CONTENT_LENGTH,
            HeaderValue::from_str(&(BODY_LIMIT_BYTES + 1).to_string()).unwrap(),
        );
        assert!(
            declared_body_length(&headers)
                .unwrap()
                .is_some_and(|length| length > BODY_LIMIT_BYTES)
        );

        headers.append(header::CONTENT_LENGTH, HeaderValue::from_static("1"));
        assert_eq!(declared_body_length(&headers), Err(()));
    }

    #[test]
    fn upload_form_decodes_exactly_one_workout_field() {
        let parsed =
            parse_upload_form(b"workout=Quickest+Arms%0ASet+1%3A+45lbs+x+10+reps").unwrap();
        assert_eq!(parsed, "Quickest Arms\nSet 1: 45lbs x 10 reps");
    }

    #[test]
    fn upload_form_rejects_missing_duplicate_unknown_and_empty_fields() {
        assert_eq!(parse_upload_form(b""), Err(UploadFormError::MissingWorkout));
        assert_eq!(
            parse_upload_form(b"workout=one&workout=two"),
            Err(UploadFormError::DuplicateWorkout)
        );
        assert_eq!(
            parse_upload_form(b"workout=one&submit=Publish"),
            Err(UploadFormError::UnexpectedField)
        );
        assert_eq!(
            parse_upload_form(b"workout=+++"),
            Err(UploadFormError::EmptyWorkout)
        );
    }

    #[test]
    fn upload_form_rejects_malformed_escapes_and_utf8() {
        assert_eq!(
            parse_upload_form(b"workout=bad%2"),
            Err(UploadFormError::BadEncoding)
        );
        assert_eq!(
            parse_upload_form(b"workout=%FF"),
            Err(UploadFormError::BadEncoding)
        );
    }

    #[test]
    fn json_negotiation_is_explicit() {
        let mut headers = HeaderMap::new();
        assert!(!wants_json(&headers));
        headers.insert(
            header::ACCEPT,
            HeaderValue::from_static("text/html, application/json; q=0.9"),
        );
        assert!(wants_json(&headers));
        headers.insert(header::ACCEPT, HeaderValue::from_static("*/*"));
        assert!(!wants_json(&headers));
    }

    #[test]
    fn cached_legacy_uploader_gets_the_alias_shape_it_accepts() {
        let path = "2026-07-24T10-38-00-04-00";
        assert_eq!(
            published_location(path, false),
            "/fitness/lift/2026-07-24T10-38-00-04-00"
        );
        assert_eq!(
            published_location(path, true),
            "/lifting/2026-07-24T10-38-00-04-00"
        );
    }

    #[test]
    fn clipboard_enhancement_uses_the_shared_server_response_and_modal_driver() {
        let upload = include_str!("interests/lifting/workout-upload.js");
        let modals = include_str!("../components/browser/modals.js");

        assert!(upload.contains("navigator.clipboard.readText()"));
        assert!(upload.contains("new ClipboardItem"));
        assert!(upload.contains("result.share_text"));
        assert!(upload.contains("window.location.assign(result.location)"));
        assert!(upload.contains("/^\\/fitness\\/lift\\/[A-Za-z0-9-]+$/"));

        // The lift script is now only a clipboard companion inside the shared
        // native dialog; the site-wide driver owns opening and focus return.
        assert!(upload.contains("document.querySelector(\"#fitness-lift-dialog\")"));
        assert!(!upload.contains("dialog.showModal()"));
        assert!(modals.contains("[data-modal-open]"));
        assert!(modals.contains("dialog.showModal()"));
        assert!(modals.contains("opener.focus()"));
    }
}
