#![doc(html_root_url = "https://docs.rs/chrome-remote-interface/0.1.0-alpha.8")]
//! [Chrome DevTools Protocol](https://chromedevtools.github.io/devtools-protocol/) client.
//!
//! Currently Work In Progress.
//!
//! ## Example
//!
//! ```
//! use chrome_remote_interface::Browser;
//! use chrome_remote_interface::model::target::{CreateTargetCommand, AttachToTargetCommand};
//!
//! #[tokio::main(flavor = "current_thread")]
//! async fn main() -> anyhow::Result<()> {
//!     pretty_env_logger::init();
//!     let browser = Browser::launcher()
//!         .headless(true) // headless mode (Default)
//!         .launch()
//!         .await?;
//!
//!     let client = browser.connect().await?;
//!     // Open new page
//!     let response = client.request(CreateTargetCommand::builder()
//!         .url("https://example.org/".into())
//!         .build()
//!         .unwrap()
//!     ).await?;
//!
//!     // Attach opened page.
//!     let response = client
//!         .request(AttachToTargetCommand::new((*response).clone(), Some(true)))
//!         .await?;
//!
//!     // construct attached session.
//!     let mut session = client.session(response);
//!
//!     // DO STUFF
//!     // ...
//!
//!     Ok(())
//! }
//! ```
//!
//! ## License
//!
//! Licensed under either of
//! * Apache License, Version 2.0
//!   ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
//! * MIT license
//!   ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)
//! at your option.
//!
//! ## Contribution
//!
//! Unless you explicitly state otherwise, any contribution intentionally submitted
//! for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
//! dual licensed as above, without any additional terms or conditions.!

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::ready;
use futures::sink::{Sink, SinkExt};
use futures::stream::{Fuse, Stream, StreamExt};
use serde_json::Value;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

pub use browser::*;
pub use chrome_remote_interface_model as model;
use model::SessionId;

/// Environment variable key for Browser Path.
pub const BROWSER_BIN: &str = "CRI_CHROME_BIN";

pub use client::{Client, Session};

macro_rules! recv {
    ($len:expr, $content:expr) => {
        log::trace!(target: "chrome_remote_interface::protocol", "<< [{} bytes] {}", $len, $content);
    }
}

macro_rules! send {
    ($len:expr, $content:expr) => {
        log::trace!(target: "chrome_remote_interface::protocol", ">> [{} bytes] {}", $len, $content);
    }
}

mod browser;
mod client;
pub(crate) mod os;
mod pipe;
pub(crate) mod process;

/// Chrome DevTools Protocol Client Error.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// IO Error.
    #[error(transparent)]
    Io(#[from] io::Error),

    /// Websocket Request Error.
    #[error(transparent)]
    WsRequest(#[from] tokio_tungstenite::tungstenite::Error),

    /// Serialize / Deserialze Error.
    #[error(transparent)]
    Json(#[from] serde_json::Error),

    /// Chrome DevTools Protocol Command Error.
    #[error("error response {0:?}")]
    Response(serde_json::Value),

    /// Browser Error.
    #[error(transparent)]
    Browser(#[from] BrowserError),

    /// Browser Error.
    #[error("transport broken: {0}")]
    TransportBroken(String),
}

/// Chrome DevTools Protocol Result.
pub type Result<T> = std::result::Result<T, Error>;

enum Channel {
    Ws(Fuse<WebSocketStream<MaybeTlsStream<TcpStream>>>),
    Pipe(pipe::PipeChannel),
}

impl Stream for Channel {
    type Item = Result<Value>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.get_mut() {
            Self::Ws(ws) => loop {
                match ready!(ws.poll_next_unpin(cx)?) {
                    Some(Message::Text(m)) => {
                        recv!(m.bytes().len(), m);
                        return Poll::Ready(Some(Ok(serde_json::from_str(&m)?)));
                    }
                    Some(Message::Binary(m)) => {
                        recv!(m.len(), String::from_utf8_lossy(&m));
                        return Poll::Ready(Some(Ok(serde_json::from_slice(&m)?)));
                    }
                    Some(..) => {}
                    None => return Poll::Ready(None),
                }
            },

            Self::Pipe(inner) => inner.poll_next_unpin(cx),
        }
    }
}

impl Sink<Value> for Channel {
    type Error = Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        match self.get_mut() {
            Self::Ws(ws) => {
                ready!(ws.poll_ready_unpin(cx)?);
                Poll::Ready(Ok(()))
            }

            Self::Pipe(inner) => inner.poll_ready_unpin(cx),
        }
    }

    fn start_send(self: Pin<&mut Self>, item: Value) -> Result<()> {
        match self.get_mut() {
            Self::Ws(ws) => {
                let item = serde_json::to_string(&item)?;
                send!(item.bytes().len(), &item);
                ws.start_send_unpin(Message::Text(item))?;
                Ok(())
            }

            Self::Pipe(inner) => inner.start_send_unpin(item),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        match self.get_mut() {
            Self::Ws(ws) => {
                ready!(ws.poll_flush_unpin(cx)?);
                Poll::Ready(Ok(()))
            }

            Self::Pipe(inner) => inner.poll_flush_unpin(cx),
        }
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        match self.get_mut() {
            Self::Ws(ws) => {
                ready!(ws.poll_close_unpin(cx)?);
                Poll::Ready(Ok(()))
            }

            Self::Pipe(inner) => inner.poll_close_unpin(cx),
        }
    }
}
