use std::net::SocketAddr;
use std::time::Duration;

use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use reqwest::{Client, Response, StatusCode};
use ring::hmac;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{ClientError, ErrorCode, SigningKey};

const SCHEME: &str = "Vespera-HMAC-SHA256";
const MAX_BODY: usize = 512 * 1024;
const TIMEOUT: Duration = Duration::from_secs(15);

pub(super) struct Transport {
    client: Client,
    pub(super) address: SocketAddr,
    pub(super) path: &'static str,
    principal: String,
    key: hmac::Key,
}

#[derive(Deserialize)]
pub(super) struct Session {
    session_id: Uuid,
    elapsed_ms: u64,
}

impl Transport {
    pub(super) fn new(
        address: SocketAddr,
        path: &'static str,
        principal: String,
        key: SigningKey,
    ) -> Result<Self, ClientError> {
        if !address.ip().is_loopback() || address.port() == 0 {
            return Err(ClientError::InvalidEndpoint);
        }
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .http1_only()
            .timeout(TIMEOUT)
            .build()
            .map_err(|_| ClientError::ClientInitialization)?;
        Ok(Self {
            client,
            address: SocketAddr::new(address.ip(), address.port()),
            path,
            principal,
            key: hmac::Key::new(hmac::HMAC_SHA256, &key.0),
        })
    }

    pub(super) async fn post(&self, action: &impl Serialize) -> Result<Reply, ClientError> {
        let body = serde_json::to_vec(action).map_err(|_| ClientError::Encoding)?;
        if body.len() > MAX_BODY {
            return Err(ClientError::RequestTooLarge);
        }
        let authorization = self.proof(self.session().await?, "POST", self.path, &body)?;
        self.client
            .post(format!("http://{}{}", self.address, self.path))
            .header(AUTHORIZATION, authorization)
            .header(CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map(Reply)
            .map_err(transport_error)
    }

    pub(super) async fn session(&self) -> Result<Session, ClientError> {
        let response = self
            .client
            .get(format!("http://{}/v2/session", self.address))
            .send()
            .await
            .map_err(transport_error)?;
        let session: Session = Reply(response).json(StatusCode::OK).await?;
        if session.session_id.is_nil() {
            return Err(ClientError::UnexpectedResponse { status: 200 });
        }
        Ok(session)
    }

    pub(super) fn proof(
        &self,
        session: Session,
        method: &str,
        target: &str,
        body: &[u8],
    ) -> Result<HeaderValue, ClientError> {
        use std::fmt::Write;

        let Session {
            session_id,
            elapsed_ms,
        } = session;
        let nonce = Uuid::new_v4();
        let mut context = hmac::Context::with_key(&self.key);
        let prefix = format!(
            "{SCHEME}\n{method}\n{target}\n{}\n{session_id}\n{elapsed_ms}\n{nonce}\n",
            self.principal
        );
        context.update(prefix.as_bytes());
        context.update(body);
        let mac = context.sign();
        let mut value = format!(
            "{SCHEME} {}:{session_id}:{elapsed_ms}:{nonce}:",
            self.principal
        );
        for byte in mac.as_ref() {
            write!(&mut value, "{byte:02x}").map_err(|_| ClientError::Encoding)?;
        }
        let mut header = HeaderValue::from_str(&value).map_err(|_| ClientError::Encoding)?;
        header.set_sensitive(true);
        Ok(header)
    }
}

pub(super) fn error_code(bytes: &[u8]) -> Option<ErrorCode> {
    #[derive(Deserialize)]
    struct ErrorBody {
        error: ErrorCode,
    }
    serde_json::from_slice::<ErrorBody>(bytes)
        .ok()
        .map(|body| body.error)
}

pub(super) struct Reply(Response);

impl Reply {
    pub(super) fn status(&self) -> StatusCode {
        self.0.status()
    }

    pub(super) async fn no_content(self) -> Result<(), ClientError> {
        if self.status() == StatusCode::NO_CONTENT {
            Ok(())
        } else {
            Err(self.error().await?)
        }
    }

    pub(super) async fn json<T: DeserializeOwned>(
        self,
        expected: StatusCode,
    ) -> Result<T, ClientError> {
        let status = self.status();
        if status != expected {
            return Err(self.error().await?);
        }
        if !self.is_json() {
            return Err(ClientError::UnexpectedResponse {
                status: status.as_u16(),
            });
        }
        let bytes = self.bytes().await?;
        serde_json::from_slice(&bytes).map_err(|_| ClientError::UnexpectedResponse {
            status: status.as_u16(),
        })
    }

    async fn error(self) -> Result<ClientError, ClientError> {
        let status = self.status();
        if !status.is_client_error() && !status.is_server_error() {
            return Ok(ClientError::UnexpectedResponse {
                status: status.as_u16(),
            });
        }
        let code = if self.is_json() {
            error_code(&self.bytes().await?)
        } else {
            None
        };
        Ok(ClientError::Rejected {
            status: status.as_u16(),
            code,
        })
    }

    fn is_json(&self) -> bool {
        self.0
            .headers()
            .get(CONTENT_TYPE)
            .is_some_and(|value| value == "application/json")
    }

    async fn bytes(mut self) -> Result<Vec<u8>, ClientError> {
        if self
            .0
            .content_length()
            .is_some_and(|length| length > MAX_BODY as u64)
        {
            return Err(ClientError::ResponseTooLarge);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = self.0.chunk().await.map_err(transport_error)? {
            if chunk.len() > MAX_BODY - bytes.len() {
                return Err(ClientError::ResponseTooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
}

fn transport_error(error: reqwest::Error) -> ClientError {
    if error.is_timeout() {
        ClientError::Timeout
    } else {
        ClientError::Transport
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OperatorClient;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use vespera_core::TaskId;

    async fn serve_session_reply(reply: Vec<u8>) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            drop(listener);
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(stream.read_u8().await.unwrap());
                assert!(request.len() <= 4096);
            }
            assert!(request.starts_with(b"GET /v2/session HTTP/1.1\r\n"));
            let _ = stream.write_all(&reply).await;
        });
        address
    }

    #[tokio::test]
    async fn rejects_redirect_from_session_endpoint() {
        let reply = b"HTTP/1.1 307 Temporary Redirect\r\nLocation: /other\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        let address = serve_session_reply(reply.to_vec()).await;
        let client = OperatorClient::new(address, SigningKey([1; 32])).unwrap();
        assert!(matches!(
            client.inspect(TaskId::new()).await,
            Err(ClientError::UnexpectedResponse { status: 307 })
        ));
    }

    #[tokio::test]
    async fn rejects_oversized_chunked_response_without_content_length() {
        let mut reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n",
            MAX_BODY + 1
        ).into_bytes();
        reply.resize(reply.len() + MAX_BODY + 1, b' ');
        reply.extend_from_slice(b"\r\n0\r\n\r\n");
        let address = serve_session_reply(reply).await;
        let client = OperatorClient::new(address, SigningKey([1; 32])).unwrap();
        assert!(matches!(
            client.inspect(TaskId::new()).await,
            Err(ClientError::ResponseTooLarge)
        ));
    }
}
