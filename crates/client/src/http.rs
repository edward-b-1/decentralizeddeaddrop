//! Requests to one server, over TLS that accepts only that server's fingerprint.

use std::time::Duration;

use reqwest::Method;
use reqwest::blocking::Client;
use serde_json::Value;

use crate::ClientError;
use crate::link::ServerAddress;

pub struct Api {
    base: String,
    client: Client,
}

impl Api {
    pub fn new(address: &ServerAddress) -> Result<Api, ClientError> {
        let tls = deaddrop_tls::pinned_client_config(&address.fingerprint).ok_or(ClientError::BadLink)?;
        let client = Client::builder()
            .tls_backend_preconfigured(tls)
            .https_only(true)
            // Straight to the server: never through whatever proxy the system suggests.
            .no_proxy()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| ClientError::Unreachable)?;
        Ok(Api { base: format!("https://{}", address.authority()), client })
    }

    /// One request. A reply that is not a success becomes [`ClientError::Refused`] with
    /// the server's own short explanation.
    pub fn call(
        &self,
        method: Method,
        path: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> Result<Value, ClientError> {
        let mut request = self.client.request(method, format!("{}{path}", self.base));
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().map_err(|_| ClientError::Unreachable)?;
        let status = response.status();
        let text = response.text().map_err(|_| ClientError::Unreachable)?;
        let json: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if status.is_success() {
            Ok(json)
        } else {
            let message = json["error"].as_str().unwrap_or("").to_owned();
            Err(ClientError::Refused { status: status.as_u16(), message })
        }
    }
}
