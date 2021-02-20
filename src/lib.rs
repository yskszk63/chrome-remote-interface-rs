#![doc(html_root_url = "https://docs.rs/chrome-remote-interface/0.1.0-alpha.5")]
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
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::channel::mpsc;
use futures::channel::oneshot;
use futures::future::FutureExt;
use futures::ready;
use futures::select;
use futures::sink::{Sink, SinkExt};
use futures::stream::{Fuse, Stream, StreamExt};
use serde::Deserialize;
use serde_json::Value;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio_tungstenite::WebSocketStream;
use url::Url;

pub use browser::*;
pub use chrome_remote_interface_model as model;
use model::SessionId;

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
pub(crate) mod os;

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
    Pipe(os::PipeChannel),
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

/// Chrome DevTools Protocol Client Command Request Future.
#[derive(Debug)]
pub struct Request<R> {
    tx_result: Option<Result<()>>,
    rx: oneshot::Receiver<std::result::Result<Value, Value>>,
    _phantom: PhantomData<R>,
}

impl<R> Future for Request<R>
where
    R: for<'r> Deserialize<'r> + Unpin,
{
    type Output = Result<R>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if let Some(r) = this.tx_result.take() {
            r?;
        }
        match ready!(this.rx.poll_unpin(cx))? {
            Ok(v) => Poll::Ready(Ok(serde_json::from_value(v)?)),
            Err(err) => Poll::Ready(Err(Error::Response(err))),
        }
    }
}

/// Chrome DevTools Protocol Session.
#[derive(Debug, Clone)]
pub struct CdpSession {
    idgen: Arc<AtomicU32>,
    session_id: Option<SessionId>,
    control_tx: mpsc::UnboundedSender<Control>,
    browser: Option<Arc<Browser>>,
}

impl CdpSession {
    /// Request for Chrome DevTools Protocol Command.
    pub fn request<C>(&mut self, command: C) -> Request<C::Return>
    where
        C: model::Command,
    {
        let id = self.idgen.fetch_add(1, Ordering::SeqCst);
        let request = command.into_request(self.session_id.clone(), id);
        let request = serde_json::to_value(&request).unwrap(); // FIXME
        let (tx, rx) = oneshot::channel();
        let tx_result = self
            .control_tx
            .unbounded_send(Control::Request(id, request, tx));

        Request {
            tx_result: Some(tx_result.map_err(Into::into)),
            rx,
            _phantom: PhantomData,
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

async fn r#loop(
    mut control_rx: mpsc::UnboundedReceiver<Control>,
    mut channel: Channel,
) -> Result<()> {
    let mut waiters = HashMap::<u32, oneshot::Sender<std::result::Result<Value, Value>>>::new();
    let mut events = HashMap::<Option<SessionId>, Vec<mpsc::UnboundedSender<model::Event>>>::new();

    loop {
        select! {
            ctrl = control_rx.next() => {
                match ctrl {
                    Some(Control::Subscribe(session_id, tx)) => {
                        events.entry(session_id).or_insert_with(Default::default).push(tx);
                    }
                    Some(Control::Request(id, request, result)) => {
                        channel.send(request).await?;
                        waiters.insert(id, result);
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

                            model::Response::Return(_, id, v) => {
                                if let Some(tx) = waiters.remove(&id) {
                                    tx.send(Ok(v)).unwrap();
                                }
                            }

                            model::Response::Error(_, id, err) => {
                                if let Some(tx) = waiters.remove(&id) {
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
        u32,
        Value,
        oneshot::Sender<std::result::Result<Value, Value>>,
    ),
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
            idgen: Arc::new(AtomicU32::default()),
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

    async fn connect_pipe(browser: Browser, channel: os::PipeChannel) -> Result<(Self, Loop)> {
        let channel = Channel::Pipe(channel);
        Ok(Self::connect_internal(channel, Some(browser)).await)
    }

    /// Construct session with target.
    pub fn session<S: Deref<Target = model::target::SessionId>>(
        &mut self,
        session_id: S,
    ) -> CdpSession {
        let session_id = Some(SessionId::from(session_id.as_ref()));
        CdpSession {
            idgen: self.idgen.clone(),
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
