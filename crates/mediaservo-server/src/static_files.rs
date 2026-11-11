use axum::Router;

// Embedded web-app dists (build.rs include_bytes!) — runtime needs NO filesystem
// dependency on dist (fixes prod-image /workspace absolute-path flaw).
// include! expands the generated `pub static FILES: &[(&str, &[u8])]` items.
// P3-3: the voice app embeds with the same posture (optional, like admin).
#[cfg(feature = "admin-dashboard")]
include!(concat!(env!("OUT_DIR"), "/embedded_admin_dist.rs"));
#[cfg(feature = "admin-dashboard")]
include!(concat!(env!("OUT_DIR"), "/embedded_voice_dist.rs"));

pub fn add_admin_routes(router: Router) -> Router {
    #[cfg(feature = "admin-dashboard")]
    {
        use axum::response::Redirect;
        use axum::routing::get;
        let router = add_embedded_spa(router, "admin", ADMIN_FILES);
        router.route("/", get(|| async { Redirect::permanent("/admin") }))
    }
    #[cfg(not(feature = "admin-dashboard"))]
    {
        router
    }
}

/// P3-3: the voice app under /voice (same embed posture as admin).
pub fn add_voice_routes(router: Router) -> Router {
    #[cfg(feature = "admin-dashboard")]
    {
        add_embedded_spa(router, "voice", VOICE_FILES)
    }
    #[cfg(not(feature = "admin-dashboard"))]
    {
        router
    }
}

/// Shared embedded-SPA surface (P3-3, generalized from add_admin_routes —
/// admin behavior preserved exactly): /{prefix}, /{prefix}/, /{prefix}/assets/*path
/// (real files) and /{prefix}/*path (SPA fallback for route refreshes).
#[cfg(feature = "admin-dashboard")]
fn add_embedded_spa(
    router: Router,
    prefix: &'static str,
    files: &'static [(&'static str, &'static [u8])],
) -> Router {
    use axum::body::Body;
    use axum::extract::Path;
    use axum::http::{HeaderValue, Response, StatusCode, header};
    use axum::response::Html;
    use axum::routing::get;
    use std::sync::Arc;

    let index_html = files
        .iter()
        .find(|(name, _)| *name == "index.html")
        .map(|(_, data)| String::from_utf8_lossy(data).to_string())
        .unwrap_or_else(|| "<html><h1>SPA not built</h1></html>".to_string());
    let assets_src = format!("src=\"/{prefix}/assets/");
    let assets_href = format!("href=\"/{prefix}/assets/");
    let html = Arc::new(
        index_html.replace("src=\"/assets/", &assets_src).replace("href=\"/assets/", &assets_href),
    );

    let index = html.clone();
    let index_trailing = html.clone();
    let index_spa = html.clone();
    let root = format!("/{prefix}");
    let root_trailing = format!("/{prefix}/");
    let assets_glob = format!("/{prefix}/assets/*path");
    let spa_glob = format!("/{prefix}/*path");
    router
        .route(
            &root,
            get(move || {
                let h = index.clone();
                async move { Html((*h).clone()) }
            }),
        )
        .route(
            &root_trailing,
            get(move || {
                let h = index_trailing.clone();
                async move { Html((*h).clone()) }
            }),
        )
        .route(
            &assets_glob,
            get(move |Path(path): Path<String>| async move {
                // matchit catch-all (/*path) captured value = the remainder after "assets"
                // (observed: "index.js"); FILES keys look like "assets/index.js" — re-prepend.
                let trimmed = path.trim_start_matches('/');
                let candidates: [String; 2] = [format!("assets/{trimmed}"), trimmed.to_string()];
                if let Some((matched, data)) = candidates
                    .iter()
                    .find_map(|k| files.iter().find(|(name, _)| name == k).map(|(n, d)| (*n, *d)))
                {
                    let content_type = content_type_for(matched);
                    let mut resp = Response::new(Body::from(data.to_vec()));
                    resp.headers_mut()
                        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
                    resp
                } else {
                    Response::builder().status(StatusCode::NOT_FOUND).body(Body::empty()).unwrap()
                }
            }),
        )
        // v3 (encode-time-stats T0): SPA fallback -- route refresh (e.g. /admin/rooms)
        // serves index.html; must be registered AFTER the assets glob (matchit order)
        .route(
            &spa_glob,
            get(move || {
                let h = index_spa.clone();
                async move { Html((*h).clone()) }
            }),
        )
}

#[cfg(feature = "admin-dashboard")]
fn content_type_for(name: &str) -> &'static str {
    if name.ends_with(".js") {
        "application/javascript"
    } else if name.ends_with(".css") {
        "text/css"
    } else if name.ends_with(".map") {
        "application/json"
    } else if name.ends_with(".svg") {
        "image/svg+xml"
    } else if name.ends_with(".png") {
        "image/png"
    } else if name.ends_with(".ico") {
        "image/x-icon"
    } else {
        "application/octet-stream"
    }
}
