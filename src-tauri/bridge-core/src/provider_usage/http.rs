use reqwest::{
    blocking::{Client, RequestBuilder},
    header::{HeaderValue, AUTHORIZATION, COOKIE},
    redirect::Policy,
};
use serde_json::Value;
use std::{io::Read, time::Duration};

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
    let response = request
        .send()
        .map_err(|_| format!("{provider} usage request failed or timed out. Try Refresh."))?;
    let status = response.status();
    if !status.is_success() {
        return Err(match status.as_u16() {
            401 | 403 => format!("Reconnect {provider} to read account usage."),
            429 => format!("{provider} is rate limited. Wait a few minutes before refreshing."),
            _ => format!(
                "{provider} usage is unavailable (HTTP {}).",
                status.as_u16()
            ),
        });
    }
    let mut bytes = Vec::new();
    response
        .take(1_048_577)
        .read_to_end(&mut bytes)
        .map_err(|_| format!("{provider} response could not be read"))?;
    if bytes.len() > 1_048_576 {
        return Err(format!("{provider} response is too large"));
    }
    String::from_utf8(bytes).map_err(|_| format!("{provider} response is not valid text"))
}
pub(super) fn json(request: RequestBuilder, provider: &str) -> Result<Value, String> {
    serde_json::from_str(&text(request, provider)?)
        .map_err(|_| format!("{provider} returned an unrecognized usage response"))
}
