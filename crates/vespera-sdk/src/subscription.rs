use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use reqwest::header::AUTHORIZATION;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::{Message, WebSocketConfig};
use tokio_tungstenite::{WebSocketStream, client_async_with_config, tungstenite};
use vespera_core::{BrokerEventCursor, BrokerStreamFrame};

use crate::ClientError;
use crate::transport::{Transport, error_code};

/// A read-only WebSocket carrying ordered events for one broker principal.
pub struct EventSubscription(WebSocketStream<Box<dyn SocketIo>>);

trait SocketIo: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> SocketIo for T {}

impl EventSubscription {
    pub(super) async fn connect(
        transport: &Transport,
        after: Option<BrokerEventCursor>,
    ) -> Result<Self, ClientError> {
        let session = transport.session().await?;
        let target = match after {
            Some(cursor) => format!(
                "{}/events?after={}:{}",
                transport.path, cursor.stream_id, cursor.sequence
            ),
            None => format!("{}/events", transport.path),
        };
        let scheme = if transport.tls.is_some() { "wss" } else { "ws" };
        let mut request = format!("{scheme}://{}{}", transport.address, target)
            .into_client_request()
            .map_err(|_| ClientError::Encoding)?;
        let authorization = transport.proof(session, "GET", &target, b"")?;
        request.headers_mut().insert(AUTHORIZATION, authorization);
        let config = WebSocketConfig::default()
            .read_buffer_size(4096)
            .write_buffer_size(0)
            .max_write_buffer_size(8192)
            .max_message_size(Some(4096))
            .max_frame_size(Some(4096));
        timeout(Duration::from_secs(15), async {
            let stream = TcpStream::connect(transport.address)
                .await
                .map_err(|_| ClientError::Transport)?;
            let stream: Box<dyn SocketIo> = if let Some(tls) = &transport.tls {
                let name = rustls::pki_types::ServerName::IpAddress(transport.address.ip().into());
                Box::new(
                    TlsConnector::from(tls.clone())
                        .connect(name, stream)
                        .await
                        .map_err(|_| ClientError::Transport)?,
                )
            } else {
                Box::new(stream)
            };
            client_async_with_config(request, stream, Some(config))
                .await
                .map(|(socket, _)| Self(socket))
                .map_err(handshake_error)
        })
        .await
        .map_err(|_| ClientError::Timeout)?
    }

    /// Receive a frame or return `None` after a clean close. Keep the cursor
    /// from each successfully processed event to resume on a new connection.
    pub async fn next(&mut self) -> Result<Option<BrokerStreamFrame>, ClientError> {
        timeout(Duration::from_secs(65), self.read())
            .await
            .map_err(|_| ClientError::Timeout)?
    }

    async fn read(&mut self) -> Result<Option<BrokerStreamFrame>, ClientError> {
        while let Some(message) = self.0.next().await {
            match message.map_err(|_| ClientError::Transport)? {
                Message::Text(text) => {
                    return serde_json::from_str(&text)
                        .map(Some)
                        .map_err(|_| ClientError::InvalidEventFrame);
                }
                Message::Close(_) => {
                    let _ = timeout(Duration::from_secs(2), self.0.flush()).await;
                    return Ok(None);
                }
                Message::Ping(_) => timeout(Duration::from_secs(2), self.0.flush())
                    .await
                    .map_err(|_| ClientError::Timeout)?
                    .map_err(|_| ClientError::Transport)?,
                Message::Pong(_) => {}
                _ => return Err(ClientError::InvalidEventFrame),
            }
        }
        Ok(None)
    }
}

fn handshake_error(error: tungstenite::Error) -> ClientError {
    match error {
        tungstenite::Error::Http(response) => ClientError::Rejected {
            status: response.status().as_u16(),
            code: response.body().as_deref().and_then(error_code),
        },
        _ => ClientError::Transport,
    }
}
