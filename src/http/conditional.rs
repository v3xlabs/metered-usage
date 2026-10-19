//! Conditional reads. A list answers with an `ETag` of its body, and a reader that sends the
//! tag back in `If-None-Match` gets 304 with no body for as long as the list reads the same.
//! The tag is taken over the bytes sent, so it changes exactly when the answer does, also
//! for parts no change records, such as a prediction that moved with usage.

use poem::http::{HeaderValue, StatusCode, header};
use poem::{Body, Endpoint, IntoResponse, Request, Response};

pub struct Conditional<E>(E);

impl<E: Endpoint> Endpoint for Conditional<E> {
    type Output = Response;

    async fn call(&self, request: Request) -> poem::Result<Self::Output> {
        // A list of tags may be split over several field lines.
        let if_none_match = request
            .headers()
            .get_all(header::IF_NONE_MATCH)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .collect::<Vec<_>>()
            .join(",");
        let response = self.0.call(request).await?.into_response();
        if response.status() != StatusCode::OK {
            return Ok(response);
        }
        let (parts, body) = response.into_parts();
        let body = body.into_bytes().await?;
        let tag = format!("\"{}\"", blake3::hash(&body).to_hex());
        if names(&if_none_match, &tag) {
            return Ok(Response::builder()
                .status(StatusCode::NOT_MODIFIED)
                .header(header::ETAG, tag)
                .finish());
        }
        let mut response = Response::from_parts(parts, Body::from(body));
        // Quotes around hex digits are always a valid header value.
        if let Ok(tag) = HeaderValue::try_from(tag) {
            response.headers_mut().insert(header::ETAG, tag);
        }

        Ok(response)
    }
}

/// The `transform` of an operation whose 200 answers are tagged. poem-openapi names a
/// function there, not a constructor.
pub fn conditional<E: Endpoint>(endpoint: E) -> Conditional<E> {
    Conditional(endpoint)
}

/// Whether an `If-None-Match` value names the tag. It compares weakly, as RFC 9110 section
/// 13.1.2 asks, so a `W/` prefix is ignored, and `*` names any tag.
fn names(header: &str, tag: &str) -> bool {
    header.split(',').map(str::trim).any(|candidate| {
        candidate == "*" || candidate.strip_prefix("W/").unwrap_or(candidate) == tag
    })
}
