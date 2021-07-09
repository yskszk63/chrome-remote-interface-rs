use std::collections::HashMap;
use std::future::Future;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use super::model;
use futures::channel::mpsc;
use futures::lock::Mutex;
use futures::stream::{SplitSink, SplitStream};
use futures::{FutureExt, SinkExt, Stream, StreamExt};
use serde_json::Value as JsonValue;
use url::Url;

struct ReceiveInner<S>
where
    S: Stream<Item = super::Result<model::Response>>,
{
    stream: S,
    wakers: Vec<Waker>,
    head: Option<super::Result<model::Response>>,
    subscribers: HashMap<Option<model::SessionId>, Vec<mpsc::UnboundedSender<model::Event>>>,
}

struct Recv<S>
where
    S: Stream<Item = super::Result<model::Response>>,
{
    inner: Arc<Mutex<ReceiveInner<S>>>,
    session_id: Option<model::SessionId>,
    id: u32,
}

impl<S> Future for Recv<S>
where
    S: Stream<Item = super::Result<model::Response>> + Unpin,
{
    type Output = super::Result<Option<JsonValue>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            let mut inner = match self.inner.lock().poll_unpin(cx) {
                Poll::Ready(inner) => inner,
                Poll::Pending => return Poll::Pending,
            };

            match &inner.head {
                Some(Ok(model::Response::Event(..))) => unreachable!(),
                Some(Ok(
                    model::Response::Return(session_id, id, _)
                    | model::Response::Error(session_id, id, _),
                )) if *session_id == self.session_id && *id == self.id => {
                    match inner.head.take().unwrap().unwrap() {
                        model::Response::Return(_, _, v) => return Poll::Ready(Ok(Some(v))),
                        model::Response::Error(_, _, v) => {
                            return Poll::Ready(Err(super::Error::Response(v)))
                        }
                        _ => unreachable!(),
                    }
                }
                Some(Ok(model::Response::Return(..) | model::Response::Error(..))) => {
                    inner.wakers.push(cx.waker().clone());
                    return Poll::Pending;
                }
                Some(Err(err)) => {
                    return Poll::Ready(Err(super::Error::TransportBroken(format!("{}", err))));
                }
                None => {}
            }

            let result = match inner.stream.poll_next_unpin(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(result) => result,
            };

            for w in inner.wakers.drain(..) {
                w.wake();
            }

            match result {
                Some(Ok(model::Response::Event(session_id, event))) => {
                    for tx in inner.subscribers.entry(session_id).or_default() {
                        tx.unbounded_send(event.clone()).ok();
                    }
                }
                Some(response) => inner.head = Some(response),
                None => {
                    inner.subscribers.clear();
                    return Poll::Ready(Ok(None));
                }
            }
        }
    }
}

struct Next<S>
where
    S: Stream<Item = super::Result<model::Response>>,
{
    inner: Arc<Mutex<ReceiveInner<S>>>,
}

impl<S> Future for Next<S>
where
    S: Stream<Item = super::Result<model::Response>> + Unpin,
{
    type Output = Option<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut inner = match self.inner.lock().poll_unpin(cx) {
            Poll::Ready(inner) => inner,
            Poll::Pending => return Poll::Pending,
        };

        let result = match inner.stream.poll_next_unpin(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(result) => result,
        };

        if inner.head.is_some() {
            inner.wakers.push(cx.waker().clone());
            return Poll::Pending;
        }

        for w in inner.wakers.drain(..) {
            w.wake();
        }

        match result {
            Some(Ok(model::Response::Event(session_id, event))) => {
                for tx in inner.subscribers.entry(session_id).or_default() {
                    tx.unbounded_send(event.clone()).ok();
                }
            }
            Some(response) => inner.head = Some(response),
            None => {
                inner.subscribers.clear();
                return Poll::Ready(None);
            }
        }
        Poll::Ready(Some(()))
    }
}

struct Receive<S>(Arc<Mutex<ReceiveInner<S>>>)
where
    S: Stream<Item = super::Result<model::Response>>;

impl<S> Clone for Receive<S>
where
    S: Stream<Item = super::Result<model::Response>>,
{
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<S> Receive<S>
where
    S: Stream<Item = super::Result<model::Response>> + Unpin,
{
    fn recv(&self, session_id: Option<model::SessionId>, id: u32) -> Recv<S> {
        Recv {
            inner: self.0.clone(),
            session_id,
            id,
        }
    }

    fn next(&self) -> Next<S> {
        Next {
            inner: self.0.clone(),
        }
    }

    async fn subscribe(
        &self,
        session_id: Option<super::SessionId>,
    ) -> mpsc::UnboundedReceiver<model::Event> {
        let (tx, rx) = mpsc::unbounded();

        let mut inner = self.0.lock().await;
        inner
            .subscribers
            .entry(session_id)
            .or_insert_with(Vec::new)
            .push(tx);
        rx
    }
}

struct CvtChannel(SplitStream<super::Channel>);

impl Stream for CvtChannel {
    type Item = super::Result<model::Response>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.get_mut().0.poll_next_unpin(cx)? {
            Poll::Ready(Some(json)) => Poll::Ready(Some(Ok(serde_json::from_value(json)?))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Stream for Chrome DevTools Protocol Event.
pub struct Events {
    receive: Receive<CvtChannel>,
    rx: mpsc::UnboundedReceiver<model::Event>,
}

impl Stream for Events {
    type Item = model::Event;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let Self { receive, rx } = self.get_mut();
        loop {
            match rx.poll_next_unpin(cx) {
                Poll::Ready(result) => return Poll::Ready(result),
                Poll::Pending => {}
            }

            match receive.next().poll_unpin(cx) {
                Poll::Ready(..) => {}
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

/// Chrome DevTools Protocol Session.
pub struct Session {
    idgen: Arc<AtomicU32>,
    session_id: Option<model::SessionId>,
    rx: Receive<CvtChannel>,
    tx: Arc<Mutex<SplitSink<super::Channel, JsonValue>>>,
    browser: Option<Arc<Mutex<super::Browser>>>,
}

impl Session {
    fn new(channel: super::Channel, browser: Option<super::Browser>) -> Self {
        let (tx, rx) = channel.split();
        let rx = CvtChannel(rx);
        let rx = Receive(Arc::new(Mutex::new(ReceiveInner {
            stream: rx,
            wakers: Default::default(),
            head: None,
            subscribers: Default::default(),
        })));
        let tx = Arc::new(Mutex::new(tx));
        let browser = browser.map(Mutex::new).map(Arc::new);
        Self {
            idgen: Arc::new(Default::default()),
            session_id: None,
            rx,
            tx,
            browser,
        }
    }

    fn session_for(&self, session_id: model::SessionId) -> Self {
        Self {
            idgen: self.idgen.clone(),
            session_id: Some(session_id),
            rx: Receive(self.rx.0.clone()),
            tx: self.tx.clone(),
            browser: self.browser.clone(),
        }
    }

    async fn request_inner<C>(
        command: C,
        tx: Arc<Mutex<SplitSink<super::Channel, JsonValue>>>,
        rx: Receive<CvtChannel>,
        session_id: Option<model::SessionId>,
        id: u32,
    ) -> super::Result<C::Return>
    where
        C: model::Command,
    {
        let request = command.into_request(session_id.clone(), id);
        let request = serde_json::to_value(request)?;

        let mut tx = tx.lock().await;
        tx.send(request).await?;

        let result = rx.recv(session_id, id).await?;
        let result = result.unwrap(); // TODO eof
        let result = serde_json::from_value(result)?;
        Ok(result)
    }

    /// Request for Chrome DevTools Protocol Command.
    pub fn request<C>(&self, command: C) -> impl Future<Output = super::Result<C::Return>> + 'static
    where
        C: model::Command + 'static,
    {
        let id = self.idgen.fetch_add(1, Ordering::SeqCst);

        let tx = self.tx.clone();
        let rx = self.rx.clone();
        let session_id = self.session_id.clone();

        Self::request_inner(command, tx, rx, session_id, id)
    }

    /// Subscribe Chrome DevTools Protocol Event.
    pub async fn events(&self) -> Events {
        let rx = self.rx.subscribe(self.session_id.clone()).await;
        Events {
            receive: Receive(self.rx.0.clone()),
            rx,
        }
    }
}

/// Chrome DevTools Protocol Client.
pub struct Client {
    session: Session,
}

impl Client {
    /// Connect with CDP Websocket.
    pub async fn connect_ws(url: &Url, browser: Option<super::Browser>) -> super::Result<Self> {
        let (ws, _) = tokio_tungstenite::connect_async(url).await?;
        let channel = super::Channel::Ws(ws.fuse());
        let session = Session::new(channel, browser);
        Ok(Self { session })
    }

    pub(crate) fn connect_pipe(browser: super::Browser, channel: super::pipe::PipeChannel) -> Self {
        let channel = super::Channel::Pipe(channel);
        let session = Session::new(channel, Some(browser));
        Self { session }
    }

    /// Construct session with target.
    pub fn session<S>(&self, session_id: S) -> Session
    where
        S: Deref<Target = model::target::SessionId>,
    {
        self.session
            .session_for(model::SessionId::from(session_id.as_ref()))
    }
}

impl Deref for Client {
    type Target = Session;
    fn deref(&self) -> &Self::Target {
        &self.session
    }
}

impl DerefMut for Client {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.session
    }
}
