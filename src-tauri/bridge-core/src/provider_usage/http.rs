use reqwest::{
    blocking::{Client, RequestBuilder},
    header::{HeaderValue, AUTHORIZATION, COOKIE},
    redirect::Policy,
};
use serde_json::Value;
use std::{io::Read, time::Duration};

#[derive(Debug)]
pub(super) struct RequestError {
    pub message: String,
    pub retryable: bool,
}

pub(super) fn client() -> Result<Client, String> {
    Client::builder()
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(4))
        .redirect(Policy::none())
        .user_agent("Bridge-MenuBar/1.0")
        .build()
        .map_err(|_| "Usage transport unavailable".into())
}
pub(super) fn secret(
    request: RequestBuilder,
    cookie: bool,
    value: &str,
) -> Result<RequestBuilder, String> {
    let mut header =
        HeaderValue::from_str(value).map_err(|_| "Invalid provider credentials".to_string())?;
    header.set_sensitive(true);
    Ok(request.header(if cookie { COOKIE } else { AUTHORIZATION }, header))
}
pub(super) fn text(request: RequestBuilder, provider: &str) -> Result<String, String> {
    text_result(request, provider).map_err(|error| error.message)
}

fn status_error(status: reqwest::StatusCode, provider: &str) -> RequestError {
    RequestError {
        message: match status.as_u16() {
            401 | 403 => format!("Reconnect {provider} to read account usage."),
            429 => format!("{provider} is rate limited. Wait a few minutes before refreshing."),
            _ => format!("{provider} usage is unavailable (HTTP {}).", status.as_u16()),
        },
        retryable: status.as_u16() == 408 || status.as_u16() == 429 || status.is_server_error(),
    }
}

fn text_result(request: RequestBuilder, provider: &str) -> Result<String, RequestError> {
    let response = request
        .send()
        .map_err(|error| RequestError {
            message: format!("{provider} usage request failed or timed out. Try Refresh."),
            retryable: error.is_timeout() || error.is_connect() || error.is_body() || error.is_request(),
        })?;
    let status = response.status();
    if !status.is_success() {
        return Err(status_error(status, provider));
    }
    let mut bytes = Vec::new();
    response
        .take(1_048_577)
        .read_to_end(&mut bytes)
        .map_err(|_| RequestError { message: format!("{provider} response could not be read"), retryable: true })?;
    if bytes.len() > 1_048_576 {
        return Err(RequestError { message: format!("{provider} response is too large"), retryable: false });
    }
    String::from_utf8(bytes).map_err(|_| RequestError { message: format!("{provider} response is not valid text"), retryable: false })
}
pub(super) fn json(request: RequestBuilder, provider: &str) -> Result<Value, String> {
    json_result(request, provider).map_err(|error| error.message)
}

pub(super) fn json_result(request: RequestBuilder, provider: &str) -> Result<Value, RequestError> {
    serde_json::from_str(&text_result(request, provider)?)
        .map_err(|_| RequestError { message: format!("{provider} returned an unrecognized usage response"), retryable: false })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_temporary_http_statuses_allow_cached_account_data() {
        for code in [408, 429, 500, 502, 503, 504] {
            assert!(status_error(reqwest::StatusCode::from_u16(code).unwrap(), "Cursor").retryable);
        }
        for code in [301, 400, 401, 403, 404, 422] {
            assert!(!status_error(reqwest::StatusCode::from_u16(code).unwrap(), "Cursor").retryable);
        }
    }
}
