use std::ffi::OsStr;
use std::path::Path as FilePath;

use poem::Response;
use poem::http::StatusCode;
use poem::web::Path;
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "web/dist/"]
struct Assets;

#[poem::handler]
pub fn serve(Path(path): Path<String>) -> Response {
    let path = path.trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    let (content_path, asset) = match Assets::get(path) {
        Some(asset) => (path, asset),
        None if !path
            .rsplit('/')
            .next()
            .is_some_and(|segment| segment.contains('.')) =>
        {
            match Assets::get("index.html") {
                Some(asset) => ("index.html", asset),
                None => return Response::builder().status(StatusCode::NOT_FOUND).finish(),
            }
        }
        None => return Response::builder().status(StatusCode::NOT_FOUND).finish(),
    };

    Response::builder()
        .content_type(content_type(content_path))
        .body(asset.data.into_owned())
}

fn content_type(path: &str) -> &'static str {
    match FilePath::new(path).extension().and_then(OsStr::to_str) {
        Some("css") => "text/css; charset=utf-8",
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}
