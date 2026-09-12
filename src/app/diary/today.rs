use super::*;
use diary_core::{today, today_store};

#[query_params(error = redirect("/diary/today"))]
struct DayQuery {
    day: Option<String>,
}

#[page("/diary/today")]
async fn daily_reflection(cx: &Cx) -> Result {
    let Some(current) = viewer(cx) else {
        return Err(redirect("/login?next=%2Fdiary%2Ftoday").into());
    };
    if !is_admin(&current.email) {
        return view! {
            ((header::CACHE_CONTROL, HeaderValue::from_static(NO_STORE)))
            not_found_page(requested: "/diary/today")
        };
    }
    let current_day = today::day_at(Timestamp::now().as_second()).expect("current day");
    let query = query_params::<DayQuery>(cx)?;
    let day = query.day.clone().unwrap_or_else(|| current_day.clone());
    if !today::valid_day(&day) || day > current_day {
        return Err(redirect("/diary/today").into());
    }
    let loaded = async {
        let db = open_db(app_context::<Data>(cx)).await?;
        Ok::<_, String>((
            today_store::snapshot(&db).await?,
            store::all_entries(&db).await?,
        ))
    }
    .await;
    let (days, cues, store_ok) = match loaded {
        Ok((snapshot, entries)) => (snapshot.days, entries, true),
        Err(_) => (Vec::new(), Vec::new(), false),
    };
    view! {
        ((header::CACHE_CONTROL, HeaderValue::from_static(NO_STORE)))
        shell(page: "Diary · Today", active: "", runtime: false, pwa: true,
            diary_core::today_views::today_page(
                day: day, current_day: current_day, days: days, cues: cues, store_ok: store_ok)
            <script type="module" src=(DIARY_JS)></script>
        )
    }
}

fn api_gate(cx: &Cx, writing: bool) -> Option<Response> {
    let Some(current) = viewer(cx) else {
        return Some(api_error(StatusCode::UNAUTHORIZED, "sign in"));
    };
    if !is_admin(&current.email) {
        return Some(api_error(StatusCode::NOT_FOUND, "not found"));
    }
    if writing && !is_same_origin(headers(cx)) {
        return Some(api_error(StatusCode::FORBIDDEN, "forbidden"));
    }
    if !has_current_schema_epoch(headers(cx)) {
        return Some(schema_epoch_mismatch());
    }
    None
}

#[route(GET "/api/diary/today")]
async fn api_today_snapshot(cx: &Cx) -> Result<Response> {
    if let Some(response) = api_gate(cx, false) {
        return Ok(response);
    }
    let day = form_urlencoded::parse(uri(cx).query().unwrap_or("").as_bytes())
        .find(|(name, _)| name == "day")
        .map(|(_, value)| value.into_owned());
    if day.as_deref().is_some_and(|day| !today::valid_day(day)) {
        return Ok(api_error(StatusCode::UNPROCESSABLE_ENTITY, "invalid day"));
    }
    let result = async {
        today_store::live_snapshot(&open_db(app_context::<Data>(cx)).await?, day.as_deref()).await
    }
    .await;
    Ok(match result {
        Ok(snapshot) => api_json(
            StatusCode::OK,
            serde_json::to_string(&snapshot).expect("snapshot serializes"),
        ),
        Err(_) => api_error(StatusCode::SERVICE_UNAVAILABLE, "store unavailable"),
    })
}

#[route(POST "/api/diary/today")]
async fn api_today_write(cx: &Cx, body: Body) -> Result<Response> {
    if let Some(response) = api_gate(cx, true) {
        return Ok(response);
    }
    if !is_json_content_type(headers(cx)) {
        return Ok(api_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "JSON required",
        ));
    }
    let bytes = match to_bytes(body, BODY_LIMIT_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return Ok(api_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "entry is too large",
            ));
        }
    };
    let command: today_store::Command = match serde_json::from_slice(&bytes) {
        Ok(command) => command,
        Err(_) => return Ok(api_error(StatusCode::BAD_REQUEST, "malformed command")),
    };
    if command.schema_epoch != diary_core::contract::CURRENT_SCHEMA_EPOCH {
        return Ok(schema_epoch_mismatch());
    }
    let db = match open_db(app_context::<Data>(cx)).await {
        Ok(db) => db,
        Err(_) => {
            return Ok(api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "store unavailable",
            ));
        }
    };
    Ok(
        match today_store::apply(&db, &command, Timestamp::now().as_second()).await {
            Ok(snapshot) => api_json(
                StatusCode::OK,
                serde_json::to_string(&snapshot).expect("snapshot serializes"),
            ),
            Err(error)
                if ["reflection-closed", "stale-reflection"]
                    .iter()
                    .any(|kind| error.contains(kind)) =>
            {
                api_error(
                    StatusCode::CONFLICT,
                    "this reflection changed elsewhere or is closed",
                )
            }
            Err(error) if error.starts_with("invalid ") => {
                api_error(StatusCode::UNPROCESSABLE_ENTITY, "invalid reflection")
            }
            Err(_) => api_error(StatusCode::SERVICE_UNAVAILABLE, "store unavailable"),
        },
    )
}
