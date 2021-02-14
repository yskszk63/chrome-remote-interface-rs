use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::sink::Sink;
use futures::Stream;
use serde_json::Value;
use tokio::process::{Child, Command};

pub const USE_PIPE_DEFAULT: bool = false;

#[derive(Debug)]
pub struct PipeChannel {}

impl PipeChannel {
    pub fn new() -> Self {
        Self {}
    }
}

impl Stream for PipeChannel {
    type Item = crate::Result<Value>;

    fn poll_next(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        todo!()
    }
}

impl Sink<Value> for PipeChannel {
    type Error = crate::Error;

    fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        todo!()
    }

    fn start_send(self: Pin<&mut Self>, _: Value) -> Result<(), Self::Error> {
        todo!()
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        todo!()
    }

    fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        todo!()
    }
}

#[derive(Debug)]
pub struct OsPipe {}

impl OsPipe {
    pub fn edit_command_and_new(_: &mut Command) -> io::Result<Self> {
        todo!()
    }
}

impl From<OsPipe> for PipeChannel {
    fn from(v: OsPipe) -> Self {
        todo!()
    }
}

pub async fn proc_kill(mut proc: Child) {
    proc.kill().await.ok();
}

pub fn proc_kill_sync(mut proc: Child) {
    proc.start_kill().ok();
}
