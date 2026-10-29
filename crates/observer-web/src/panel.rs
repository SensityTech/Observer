use std::fmt::Write as _;
use std::path::Path;

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

/// The built panel (`web/dist`). Debug builds read it from disk; release builds carry it.
#[derive(RustEmbed)]
#[folder = "../../web/dist"]
#[allow_missing = true]
struct Assets;

/// What only a signed-in user reads, through the API. `cargo xtask web` writes it beside the
/// panel rather than in it: no spelling of a panel path can reach a file the panel does not hold.
#[derive(RustEmbed)]
#[folder = "../../web/private"]
#[allow_missing = true]
struct Private;

const INDEX: &str = "index.html";
const LICENCES: &str = "licences.json";

/// Vite fingerprints everything under `assets/`, so those files never change under a given name.
const IMMUTABLE: &str = "public, max-age=31536000, immutable";

pub(crate) async fn serve(uri: Uri, headers: HeaderMap) -> Response {
    let path = match uri.path().trim_start_matches('/') {
        "" => INDEX,
        path => path,
    };
    if let Some(response) = asset::<Assets>(path, &headers) {
        return response;
    }
    // Panel routes have no extension and get the index; a missing file with one is a real 404.
    if Path::new(path).extension().is_some() {
        return StatusCode::NOT_FOUND.into_response();
    }
    asset::<Assets>(INDEX, &headers).unwrap_or_else(not_built)
}

/// The third-party licence list, when this build carries one.
pub(crate) fn licences(request: &HeaderMap) -> Option<Response> {
    asset::<Private>(LICENCES, request)
}

fn asset<Files: RustEmbed>(path: &str, request: &HeaderMap) -> Option<Response> {
    let file = Files::get(path)?;
    let etag = etag(&file.metadata.sha256_hash());
    let cache_control = if path.starts_with("assets/") {
        IMMUTABLE
    } else {
        "no-cache"
    };

    let mut response = if request
        .get(header::IF_NONE_MATCH)
        .is_some_and(|value| value.as_bytes() == etag.as_bytes())
    {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        let (data, encoding) = precompressed::<Files>(path, request).unwrap_or((file.data, None));
        let mut response = Body::from(data).into_response();
        let headers = response.headers_mut();
        if let Ok(mime) = HeaderValue::from_str(file.metadata.mimetype()) {
            headers.insert(header::CONTENT_TYPE, mime);
        }
        if let Some(encoding) = encoding {
            headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static(encoding));
        }
        response
    };

    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache_control));
    headers.insert(header::VARY, HeaderValue::from_static("accept-encoding"));
    if let Ok(etag) = HeaderValue::from_str(&etag) {
        headers.insert(header::ETAG, etag);
    }
    Some(response)
}

/// The Brotli or gzip sibling the build wrote next to a file, when the client takes it.
fn precompressed<Files: RustEmbed>(
    path: &str,
    request: &HeaderMap,
) -> Option<(std::borrow::Cow<'static, [u8]>, Option<&'static str>)> {
    [("br", "br"), ("gzip", "gz")]
        .into_iter()
        .filter(|(encoding, _)| accepts(request, encoding))
        .find_map(|(encoding, extension)| {
            Files::get(&format!("{path}.{extension}")).map(|file| (file.data, Some(encoding)))
        })
}

fn accepts(request: &HeaderMap, encoding: &str) -> bool {
    let Some(accepted) = request
        .get(header::ACCEPT_ENCODING)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    accepted.split(',').any(|item| {
        let mut params = item.split(';');
        let name = params.next().unwrap_or_default().trim();
        // `q=0` (or `q=0.0`) is an explicit refusal.
        let refused = params.any(|param| {
            param
                .trim()
                .strip_prefix("q=")
                .and_then(|q| q.trim().parse::<f32>().ok())
                .is_some_and(|q| q <= 0.0)
        });
        name.eq_ignore_ascii_case(encoding) && !refused
    })
}

fn etag(hash: &[u8]) -> String {
    let mut tag = String::with_capacity(hash.len() * 2 + 2);
    tag.push('"');
    for byte in hash {
        let _ = write!(tag, "{byte:02x}");
    }
    tag.push('"');
    tag
}

fn not_built() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        "The web panel is not built into this binary. Run `cargo xtask web`, then build Observer again.\n",
    )
        .into_response()
}
