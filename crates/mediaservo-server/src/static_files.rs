use axum::Router;

// Embedded admin dist (build.rs include_bytes!) — runtime needs NO filesystem
// dependency on dist (fixes prod-image /workspace absolute-path flaw).
// include! expands the generated `pub static FILES: &[(&str, &[u8])]` items.
#[cfg(feature = "admin-dashboard")]
include!(concat!(env!("OUT_DIR"), "/embedded_admin_dist.rs"));

pub fn add_admin_routes(router: Router) -> Router {
    #[cfg(feature = "admin-dashboard")]
    {
        use axum::body::Body;
        use axum::extract::Path;
        use axum::http::{HeaderValue, Response, StatusCode, header};
        use axum::response::Redirect;
        use axum::response::{Html, IntoResponse};
        use axum::routing::get;

        let index_html = FILES
            .iter()
            .find(|(name, _)| *name == "index.html")
            .map(|(_, data)| String::from_utf8_lossy(data).to_string())
            .unwrap_or_else(|| "<html><h1>SPA not built</h1></html>".to_string());
        let html = std::sync::Arc::new(
            index_html
                .replace("src=\"/assets/", "src=\"/admin/assets/")
                .replace("href=\"/assets/", "href=\"/admin/assets/"),
        );

        async fn serve_file(path: &str) -> Response<Body> {
            // matchit catch-all (/*path) captured value = the remainder after "assets" (observed: "index.js")
            // FILES keys look like "assets/index.js" -- need to re-prepend the assets/ prefix
            let trimmed = path.trim_start_matches('/');
            let candidates: [String; 2] = [format!("assets/{trimmed}"), trimmed.to_string()];

            if let Some((_, data)) =
                candidates.iter().find_map(|k| FILES.iter().find(|(name, _)| *name == k))
            {
                let matched =
                    candidates.iter().find(|k| FILES.iter().any(|(name, _)| name == k)).unwrap();
                let content_type = if matched.ends_with(".js") {
                    "application/javascript"
                } else if matched.ends_with(".css") {
                    "text/css"
                } else if matched.ends_with(".map") {
                    "application/json"
                } else if matched.ends_with(".svg") {
                    "image/svg+xml"
                } else if matched.ends_with(".png") {
                    "image/png"
                } else if matched.ends_with(".ico") {
                    "image/x-icon"
                } else {
                    "application/octet-stream"
                };
                let mut resp = Response::new(Body::from(data.to_vec()));
                resp.headers_mut()
                    .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
                resp
            } else {
                Response::builder().status(StatusCode::NOT_FOUND).body(Body::empty()).unwrap()
            }
        }

        let index = html.clone();
        let index_trailing = html.clone();
        let index_spa = html.clone();
        router
            .route("/", get(|| async { Redirect::permanent("/admin") }))
            .route(
                "/admin",
                get(move || {
                    let h = index.clone();
                    async move { Html((*h).clone()) }
                }),
            )
            .route(
                "/admin/",
                get(move || {
                    let h = index_trailing.clone();
                    async move { Html((*h).clone()) }
                }),
            )
            .route(
                "/admin/assets/*path",
                get(|Path(path): Path<String>| async move { serve_file(&path).await }),
            )
            // v3 (encode-time-stats T0): SPA fallback -- React Router route refresh (e.g. /admin/rooms)
            // serves index.html; must be registered AFTER /admin/assets/*path (matchit prefix-match order)
            .route(
                "/admin/*path",
                get(move || {
                    let h = index_spa.clone();
                    async move { Html((*h).clone()) }
                }),
            )
    }
    #[cfg(not(feature = "admin-dashboard"))]
    {
        router
    }
}
