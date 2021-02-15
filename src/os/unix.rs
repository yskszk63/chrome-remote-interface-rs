use std::io;
use std::os::unix::io::IntoRawFd;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

use bytes::{Buf, BytesMut};
use futures::sink::Sink;
use futures::{ready, FutureExt, Stream};
use nix::fcntl::{fcntl, FcntlArg, OFlag};
use nix::sys::signal::{kill, SIGTERM};
use nix::sys::wait::waitpid;
use nix::unistd::Pid;
use nix::unistd::{close, dup, dup2};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio_pipe::{PipeRead, PipeWrite};

pub const USE_PIPE_DEFAULT: bool = !cfg!(target_os = "macos");

#[derive(Debug)]
pub struct PipeChannel {
    pipein: PipeWrite,
    pipeout: BufReader<PipeRead>,
    wbuf: BytesMut,
    rbuf: Vec<u8>,
    wakers: Vec<Waker>,
}

impl PipeChannel {
    pub fn new(pipein: PipeWrite, pipeout: PipeRead) -> Self {
        Self {
            pipein,
            pipeout: BufReader::new(pipeout),
            wbuf: Default::default(),
            rbuf: Default::default(),
            wakers: Default::default(),
        }
    }
}

impl Stream for PipeChannel {
    type Item = crate::Result<Value>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let Self { pipeout, rbuf, .. } = self.get_mut();
        let fut = pipeout.read_until(0, rbuf);
        tokio::pin!(fut);
        if ready!(fut.poll_unpin(cx)?) == 0 {
            Poll::Ready(None)
        } else {
            let b = &rbuf[..rbuf.len() - 1];
            recv!(b.len(), String::from_utf8_lossy(b));
            let v = serde_json::from_slice(b)?;
            rbuf.clear();
            Poll::Ready(Some(Ok(v)))
        }
    }
}

impl Sink<Value> for PipeChannel {
    type Error = crate::Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let Self { wbuf, wakers, .. } = self.get_mut();
        if wbuf.has_remaining() {
            let waker = cx.waker().clone();
            wakers.push(waker);
            Poll::Pending
        } else {
            Poll::Ready(Ok(()))
        }
    }

    fn start_send(self: Pin<&mut Self>, item: Value) -> Result<(), Self::Error> {
        let Self { wbuf, .. } = self.get_mut();
        let item = serde_json::to_vec(&item)?;
        send!(item.len(), String::from_utf8_lossy(&item));
        wbuf.extend_from_slice(&item);
        wbuf.extend_from_slice(&[0]);
        Ok(())
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let Self {
            wbuf,
            wakers,
            pipein,
            ..
        } = self.get_mut();

        while wbuf.has_remaining() {
            let fut = pipein.write_buf(wbuf);
            tokio::pin!(fut);
            ready!(fut.poll_unpin(cx))?;
        }

        for w in wakers.drain(..) {
            w.wake();
        }
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.poll_flush(cx)
    }
}

#[derive(Debug)]
pub struct OsPipe {
    pipein: PipeWrite,
    pipeout: PipeRead,
}

impl OsPipe {
    pub fn edit_command_and_new(command: &mut Command) -> io::Result<Self> {
        let (pipein_rx, pipein) = tokio_pipe::pipe()?;
        let (pipeout, pipeout_tx) = tokio_pipe::pipe()?;
        let pipein_rx = pipein_rx.into_raw_fd();
        let pipeout_tx = pipeout_tx.into_raw_fd();

        let flag = fcntl(pipein_rx, FcntlArg::F_GETFL)
            .map_err(|e| io::Error::from(e.as_errno().unwrap()))?;
        fcntl(
            pipein_rx,
            FcntlArg::F_SETFL(OFlag::from_bits_truncate(flag) ^ OFlag::O_NONBLOCK),
        )
        .map_err(|e| io::Error::from(e.as_errno().unwrap()))?;
        let flag = fcntl(pipeout_tx, FcntlArg::F_GETFL)
            .map_err(|e| io::Error::from(e.as_errno().unwrap()))?;
        fcntl(
            pipeout_tx,
            FcntlArg::F_SETFL(OFlag::from_bits_truncate(flag) ^ OFlag::O_NONBLOCK),
        )
        .map_err(|e| io::Error::from(e.as_errno().unwrap()))?;

        command.arg("--remote-debugging-pipe");
        unsafe {
            command.pre_exec(move || {
                let pipein2 = dup(pipein_rx).map_err(|e| io::Error::from(e.as_errno().unwrap()))?;
                let pipeout2 =
                    dup(pipeout_tx).map_err(|e| io::Error::from(e.as_errno().unwrap()))?;
                dup2(pipein2, 3).map_err(|e| io::Error::from(e.as_errno().unwrap()))?;
                dup2(pipeout2, 4).map_err(|e| io::Error::from(e.as_errno().unwrap()))?;
                close(pipein2).map_err(|e| io::Error::from(e.as_errno().unwrap()))?;
                close(pipeout2).map_err(|e| io::Error::from(e.as_errno().unwrap()))?;
                Ok(())
            });
        }

        Ok(Self { pipein, pipeout })
    }
}

impl From<OsPipe> for PipeChannel {
    fn from(v: OsPipe) -> Self {
        PipeChannel::new(v.pipein, v.pipeout)
    }
}

pub async fn proc_kill(mut proc: Child) {
    if let Some(pid) = proc.id() {
        let pid = Pid::from_raw(pid as i32);
        kill(pid, Some(SIGTERM)).ok(); // FIXME
        proc.wait().await.ok();
    }
}

pub fn proc_kill_sync(proc: Child) {
    if let Some(pid) = proc.id() {
        let pid = Pid::from_raw(pid as i32);
        kill(pid, Some(SIGTERM)).ok(); // FIXME
        waitpid(Some(pid), None).ok(); // FIXME blocking
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_large_data() {
        use crate::model::target::CreateTargetCommand;
        use crate::model::Command;
        use futures::sink::SinkExt;
        use tokio::io::AsyncReadExt;

        let (mut rx, pipein) = tokio_pipe::pipe().unwrap();
        let (pipeout, _) = tokio_pipe::pipe().unwrap();

        let task1 = async move {
            let mut channel = PipeChannel::new(pipein, pipeout);
            let url = vec!['a'; 541858].into_iter().collect();
            let data = CreateTargetCommand::builder().url(url).build().unwrap();
            let data = data.into_request(None, 0);
            let data = serde_json::to_value(data).unwrap();

            let mut result = serde_json::to_vec(&data).unwrap();
            result.push(0);
            channel.send(data).await.unwrap();
            result
        };
        let task2 = async move {
            let mut buf = vec![];
            rx.read_to_end(&mut buf).await.unwrap();
            buf
        };

        let (expect, actual) = tokio::join!(task1, task2);
        assert_eq!(String::from_utf8(expect), String::from_utf8(actual));
    }
}
