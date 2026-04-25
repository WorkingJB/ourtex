//! Shared HTTP client + config. All `ourtex-sync` calls go through
//! `RemoteClient::request_json`, which attaches the bearer token,
//! parses the server's structured error envelope, and maps common
//! tags (`unauthorized`, `not_found`, `conflict:version_conflict`)
//! onto the matching `SyncError` variants.

use crate::error::{Result, SyncError};
use reqwest::{Method, RequestBuilder, StatusCode};
use serde::{de::DeserializeOwned, Serialize};
use url::Url;
use uuid::Uuid;

/// Configuration for a remote workspace.
#[derive(Debug, Clone)]
pub struct RemoteConfig {
    pub server_url: Url,
    pub tenant_id: Uuid,
    pub session_token: String,
}

impl RemoteConfig {
    /// Path under `/v1/t/{tid}` for a tenant-scoped endpoint.
    pub(crate) fn tenant_url(&self, suffix: &str) -> Result<Url> {
        let joined = format!("v1/t/{}/", self.tenant_id);
        let base = self.server_url.join(&joined)?;
        Ok(base.join(suffix.trim_start_matches('/'))?)
    }
}

#[derive(Clone)]
pub struct RemoteClient {
    pub(crate) http: reqwest::Client,
    pub(crate) config: RemoteConfig,
}

impl RemoteClient {
    pub fn new(config: RemoteConfig) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(concat!("ourtex-sync/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("reqwest client should build with default config");
        Self { http, config }
    }

    pub fn config(&self) -> &RemoteConfig {
        &self.config
    }

    /// Send a request and deserialize the JSON response body. Non-2xx
    /// responses are converted to `SyncError::Server` (or a more
    /// specific variant when the tag matches).
    pub(crate) async fn request_json<Req, Resp>(
        &self,
        method: Method,
        url: Url,
        body: Option<&Req>,
    ) -> Result<Resp>
    where
        Req: Serialize + ?Sized,
        Resp: DeserializeOwned,
    {
        let builder = self.apply_auth(self.http.request(method, url));
        let builder = if let Some(body) = body {
            builder.json(body)
        } else {
            builder
        };
        let resp = builder.send().await?;
        let status = resp.status();
        if status.is_success() {
            Ok(resp.json().await?)
        } else {
            Err(translate_error(status, resp).await)
        }
    }

    /// Same as `request_json` but returns `()` — for DELETE and other
    /// 204-No-Content endpoints.
    pub(crate) async fn request_empty(
        &self,
        method: Method,
        url: Url,
    ) -> Result<()> {
        let resp = self.apply_auth(self.http.request(method, url)).send().await?;
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            Err(translate_error(status, resp).await)
        }
    }

    fn apply_auth(&self, builder: RequestBuilder) -> RequestBuilder {
        builder.bearer_auth(&self.config.session_token)
    }
}

/// Parse the `{ error: { tag, message } }` envelope the server uses and
/// promote common tags to typed variants. Unknown tags fall through to
/// `Server` so the caller can display them verbatim.
pub(crate) async fn translate_error(status: StatusCode, resp: reqwest::Response) -> SyncError {
    #[derive(serde::Deserialize)]
    struct Env {
        error: Detail,
    }
    #[derive(serde::Deserialize)]
    struct Detail {
        tag: String,
        message: String,
    }

    let (tag, message) = match resp.json::<Env>().await {
        Ok(env) => (env.error.tag, env.error.message),
        Err(_) => (String::from("server_error"), format!("HTTP {}", status.as_u16())),
    };

    if status == StatusCode::UNAUTHORIZED {
        return SyncError::Unauthorized;
    }
    if status == StatusCode::NOT_FOUND && tag == "not_found" {
        return SyncError::NotFound;
    }
    // Writes that hit a stale base_version come back as `conflict`
    // with message `version_conflict`. Surfacing this as a distinct
    // variant lets the desktop render the conflict resolution UI
    // without string-matching.
    if status == StatusCode::CONFLICT && message == "version_conflict" {
        return SyncError::VersionConflict;
    }
    if status == StatusCode::BAD_REQUEST {
        return SyncError::InvalidArgument(message);
    }
    SyncError::Server {
        status: status.as_u16(),
        tag,
        message,
    }
}
