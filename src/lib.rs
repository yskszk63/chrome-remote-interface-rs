#![doc(html_root_url = "https://docs.rs/chrome-remote-interface/0.1.0-alpha.2")]
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
//!     let browser = Browser::launcher()
//!         .headless(true) // headless mode (Default)
//!         .launch()
//!         .await?;
//!
//!     let (mut client, task) = browser.connect().await?;
//!     tokio::spawn(task); // Spawn message loop.
//!
//!     // Open new page
//!     let response = client.request(CreateTargetCommand::builder()
//!             .url("https://example.org/".into())
//!             .build()
//!             .unwrap()
//!         )
//!         .await?;
//!
//!     // Attach opened page.
//!     let response = client
//!         .request(AttachToTargetCommand::new((*response).clone(), Some(true)))
//!         .await?;
//!
//!     // construct attached session.
//!     let mut session = client.session(response);
//!     // DO STUFF
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

use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::sync::Arc;
#[cfg(unix)]
use std::task::Waker;
use std::task::{Context, Poll};

#[cfg(unix)]
use bytes::{Buf, BytesMut};
use futures::channel::mpsc;
use futures::channel::oneshot;
use futures::future::FutureExt;
use futures::ready;
use futures::select;
use futures::sink::{Sink, SinkExt};
use futures::stream::{Fuse, Stream, StreamExt};
use serde_json::Value;
#[cfg(unix)]
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
#[cfg(unix)]
use tokio_pipe::{PipeRead, PipeWrite};
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio_tungstenite::WebSocketStream;
use url::Url;

pub use browser::*;
pub use chrome_remote_interface_model as model;
use model::SessionId;

mod browser;

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

    /// Loop Cancelation Error.
    #[error("loop canceled")]
    LoopCanceled(#[from] oneshot::Canceled),

    /// Maybe Loop Aborted Error.
    #[error("loop aborted")]
    LoopAborted(#[from] mpsc::SendError),

    /// Browser Error.
    #[error(transparent)]
    Browser(#[from] BrowserError),
}

impl<T> From<mpsc::TrySendError<T>> for Error {
    fn from(v: mpsc::TrySendError<T>) -> Self {
        Self::LoopAborted(v.into_send_error())
    }
}

/// Chrome DevTools Protocol Result.
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
enum Channel {
    Ws(Fuse<WebSocketStream<TcpStream>>),
    #[cfg(unix)]
    Pipe {
        pipein: PipeWrite,
        pipeout: BufReader<PipeRead>,
        wbuf: BytesMut,
        rbuf: Vec<u8>,
        wakers: Vec<Waker>,
    },
}

#[cfg(unix)]
impl Channel {
    fn new_pipe(pipein: PipeWrite, pipeout: PipeRead) -> Self {
        Channel::Pipe {
            pipein,
            pipeout: BufReader::new(pipeout),
            wbuf: Default::default(),
            rbuf: Default::default(),
            wakers: Default::default(),
        }
    }
}

impl Stream for Channel {
    type Item = Result<Value>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.get_mut() {
            Self::Ws(ws) => loop {
                match ready!(ws.poll_next_unpin(cx)?) {
                    Some(Message::Text(m)) => {
                        return Poll::Ready(Some(Ok(serde_json::from_str(&m)?)))
                    }
                    Some(Message::Binary(m)) => {
                        return Poll::Ready(Some(Ok(serde_json::from_slice(&m)?)))
                    }
                    Some(..) => {}
                    None => return Poll::Ready(None),
                }
            },

            #[cfg(unix)]
            Self::Pipe { pipeout, rbuf, .. } => {
                let fut = pipeout.read_until(0, rbuf);
                tokio::pin!(fut);
                if ready!(fut.poll_unpin(cx)?) == 0 {
                    Poll::Ready(None)
                } else {
                    let v = serde_json::from_slice(&rbuf[..rbuf.len() - 1])?;
                    rbuf.clear();
                    Poll::Ready(Some(Ok(v)))
                }
            }
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

            #[cfg(unix)]
            Self::Pipe { wbuf, wakers, .. } => {
                if wbuf.has_remaining() {
                    let waker = cx.waker().clone();
                    wakers.push(waker);
                    Poll::Pending
                } else {
                    Poll::Ready(Ok(()))
                }
            }
        }
    }

    fn start_send(self: Pin<&mut Self>, item: Value) -> Result<()> {
        match self.get_mut() {
            Self::Ws(ws) => {
                let item = serde_json::to_string(&item)?;
                ws.start_send_unpin(Message::Text(item))?;
                Ok(())
            }

            #[cfg(unix)]
            Self::Pipe { wbuf, .. } => {
                wbuf.extend_from_slice(&serde_json::to_vec(&item)?);
                wbuf.extend_from_slice(&[0]);
                Ok(())
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        match self.get_mut() {
            Self::Ws(ws) => {
                ready!(ws.poll_flush_unpin(cx)?);
                Poll::Ready(Ok(()))
            }

            #[cfg(unix)]
            Self::Pipe {
                wbuf,
                pipein,
                wakers,
                ..
            } => {
                let fut = pipein.write_all(wbuf);
                tokio::pin!(fut);
                ready!(fut.poll(cx)?);
                for w in wakers.drain(..) {
                    w.wake();
                }
                wbuf.clear();
                Poll::Ready(Ok(()))
            }
        }
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        match self.get_mut() {
            Self::Ws(ws) => {
                ready!(ws.poll_close_unpin(cx)?);
                Poll::Ready(Ok(()))
            }

            #[cfg(unix)]
            this @ Self::Pipe { .. } => Pin::new(this).poll_flush(cx),
        }
    }
}

/// Stream for Chrome DevTools Protocol Event.
#[derive(Debug)]
pub struct CdpEvents {
    rx: mpsc::UnboundedReceiver<model::Event>,
}

impl Stream for CdpEvents {
    type Item = model::Event;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx.poll_next_unpin(cx)
    }
}

/// Chrome DevTools Protocol Session.
#[derive(Debug, Clone)]
pub struct CdpSession {
    session_id: Option<SessionId>,
    control_tx: mpsc::UnboundedSender<Control>,
    browser: Option<Arc<Browser>>,
}

impl CdpSession {
    /// Request for Chrome DevTools Protocol Command.
    pub async fn request<C>(&mut self, command: C) -> Result<C::Return>
    where
        C: model::Command,
    {
        let id = 0; // TODO increment
        let request = command.into_request(self.session_id.clone(), id);
        let request = serde_json::to_value(&request)?;
        let (tx, rx) = oneshot::channel();
        self.control_tx.unbounded_send(Control::Request(
            self.session_id.clone(),
            id,
            request,
            tx,
        ))?;

        match rx.await? {
            Ok(v) => Ok(serde_json::from_value(v)?),
            Err(e) => Err(Error::Response(e)),
        }
    }

    /// Subscribe Chrome DevTools Protocol Event.
    pub fn events(&mut self) -> Result<CdpEvents> {
        let (tx, rx) = mpsc::unbounded();
        self.control_tx
            .unbounded_send(Control::Subscribe(self.session_id.clone(), tx))?;
        Ok(CdpEvents { rx })
    }
}

impl Drop for CdpSession {
    fn drop(&mut self) {
        self.control_tx
            .unbounded_send(Control::Disconnect(self.session_id.clone()))
            .ok();
    }
}

async fn r#loop(
    mut control_rx: mpsc::UnboundedReceiver<Control>,
    mut channel: Channel,
) -> Result<()> {
    let mut waiters = HashMap::<
        (Option<SessionId>, u32),
        oneshot::Sender<std::result::Result<Value, Value>>,
    >::new();
    let mut events = HashMap::<Option<SessionId>, Vec<mpsc::UnboundedSender<model::Event>>>::new();

    loop {
        select! {
            ctrl = control_rx.next() => {
                log::trace!("Control message received. {:?}", ctrl);
                match ctrl {
                    Some(Control::Subscribe(session_id, tx)) => {
                        events.entry(session_id).or_insert_with(Default::default).push(tx);
                    }
                    Some(Control::Request(session_id, id, request, result)) => {
                        channel.send(request).await?;
                        waiters.insert((session_id, id), result);
                    }
                    Some(Control::Disconnect(session_id)) => {
                        waiters = waiters.drain().filter(|((s, _), _)| s != &session_id).collect();
                        events.remove(&session_id);
                    }
                    None => break,
                }
            },

            msg = channel.next().fuse() => {
                match msg {
                    Some(Ok(msg)) => {
                        let msg = serde_json::from_value(msg)?;
                        match msg {
                            model::Response::Event(session_id, evt) => {
                                for tx in &mut *events.entry(session_id).or_default() {
                                    tx.unbounded_send(evt.clone()).ok(); // TODO remove
                                }
                            }

                            model::Response::Return(session_id, id, v) => {
                                if let Some(tx) = waiters.remove(&(session_id, id)) {
                                    tx.send(Ok(v)).unwrap();
                                }
                            }

                            model::Response::Error(session_id, id, err) => {
                                if let Some(tx) = waiters.remove(&(session_id, id)) {
                                    tx.send(Err(err)).unwrap();
                                }
                            }
                        }
                    }
                    Some(Err(err)) => todo!("{:?}", err),
                    None => {}
                }
            }
        }
    }

    Ok(())
}

#[derive(Debug)]
enum Control {
    Subscribe(Option<SessionId>, mpsc::UnboundedSender<model::Event>),
    Request(
        Option<SessionId>,
        u32,
        Value,
        oneshot::Sender<std::result::Result<Value, Value>>,
    ),
    Disconnect(Option<SessionId>),
}

/// Message loop.
pub struct Loop {
    future: Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>>,
}

impl Future for Loop {
    type Output = Result<()>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.future.poll_unpin(cx)
    }
}

/// Chrome DevTools Protocol Client.
///
/// This serve as browser session.
#[derive(Debug)]
pub struct CdpClient {
    control_tx: mpsc::UnboundedSender<Control>,
    session: CdpSession,
}

impl CdpClient {
    async fn connect_internal(channel: Channel, browser: Option<Browser>) -> (Self, Loop) {
        let (control_tx, control_rx) = mpsc::unbounded();
        let task = Loop {
            future: Box::pin(r#loop(control_rx, channel)),
        };
        let browser = browser.map(Arc::new);
        let session = CdpSession {
            session_id: None,
            control_tx: control_tx.clone(),
            browser,
        };
        let client = CdpClient {
            control_tx,
            session,
        };
        (client, task)
    }

    /// Connect with CDP Websocket.
    pub async fn connect(url: &Url) -> Result<(Self, impl Future<Output = Result<()>>)> {
        Self::connect_ws(url, None).await
    }

    async fn connect_ws(url: &Url, browser: Option<Browser>) -> Result<(Self, Loop)> {
        let (ws, _) = tokio_tungstenite::connect_async(url).await?;
        let channel = Channel::Ws(ws.fuse());
        Ok(Self::connect_internal(channel, browser).await)
    }

    #[cfg(unix)]
    async fn connect_pipe(
        browser: Browser,
        pipein: PipeWrite,
        pipeout: PipeRead,
    ) -> Result<(Self, Loop)> {
        let channel = Channel::new_pipe(pipein, pipeout);
        Ok(Self::connect_internal(channel, Some(browser)).await)
    }

    /// Construct session with target.
    pub fn session<S: Deref<Target = model::target::SessionId>>(
        &mut self,
        session_id: S,
    ) -> CdpSession {
        let session_id = Some(SessionId::from(session_id.as_ref()));
        CdpSession {
            session_id,
            control_tx: self.control_tx.clone(),
            browser: self.browser.clone(),
        }
    }
}

impl Deref for CdpClient {
    type Target = CdpSession;
    fn deref(&self) -> &Self::Target {
        &self.session
    }
}

impl DerefMut for CdpClient {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.session
    }
}
