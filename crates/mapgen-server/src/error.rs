//! Errors as RFC 9457 `application/problem+json`.

use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq)]
pub struct ApiError {
    pub status: StatusCode,
    pub title: &'static str,
    pub detail: String,
    /// The offending parameter or field, when there is one.
    pub param: Option<String>,
    /// Extra members (e.g. a reshape's conflicts).
    pub extra: Option<Box<(&'static str, Value)>>,
    /// Seconds, for 503.
    pub retry_after: Option<u32>,
}

impl ApiError {
    pub fn new(status: StatusCode, title: &'static str, detail: impl Into<String>) -> Self {
        ApiError {
            status,
            title,
            detail: detail.into(),
            param: None,
            extra: None,
            retry_after: None,
        }
    }

    pub fn bad_request(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "Invalid request", detail)
    }

    pub fn bad_param(param: &str, why: &str) -> Self {
        ApiError {
            param: Some(param.to_owned()),
            ..Self::bad_request(format!("`{param}`: {why}"))
        }
    }

    pub fn not_found(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "Not found", detail)
    }

    pub fn busy(detail: impl Into<String>) -> Self {
        ApiError {
            retry_after: Some(5),
            ..Self::new(StatusCode::SERVICE_UNAVAILABLE, "Busy", detail)
        }
    }

    pub fn internal(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "Internal error", detail)
    }

    /// A render spec error, with field names as the query parameters spell
    /// them when the spec came from a query string (`css-vars`, not `cssVars`).
    pub fn spec(detail: &str, from_query: bool) -> Self {
        let detail = if from_query {
            kebab_fields(detail)
        } else {
            detail.to_owned()
        };
        Self::bad_request(detail)
    }
}

/// `unknown field `colourWater`` → `colour-water`.
fn kebab_fields(s: &str) -> String {
    let mut out = String::new();
    for (i, part) in s.split('`').enumerate() {
        if i > 0 {
            out.push('`');
        }
        if i % 2 == 1 && part.chars().all(|c| c.is_ascii_alphanumeric()) {
            for c in part.chars() {
                if c.is_ascii_uppercase() {
                    out.push('-');
                    out.push(c.to_ascii_lowercase());
                } else {
                    out.push(c);
                }
            }
        } else {
            out.push_str(part);
        }
    }
    out
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut body = json!({
            "type": "about:blank",
            "title": self.title,
            "status": self.status.as_u16(),
            "detail": self.detail,
        });
        if let Some(p) = &self.param {
            body["param"] = json!(p);
        }
        if let Some((k, v)) = self.extra.map(|e| *e) {
            body[k] = v;
        }
        let mut res = (self.status, body.to_string()).into_response();
        res.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/problem+json"),
        );
        if let Some(s) = self.retry_after {
            res.headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(s));
        }
        res
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn field_names_follow_the_query_spelling() {
        assert_eq!(
            super::kebab_fields("unknown field `colourWater`, expected one of `region`, `cssVars`"),
            "unknown field `colour-water`, expected one of `region`, `css-vars`"
        );
    }
}
