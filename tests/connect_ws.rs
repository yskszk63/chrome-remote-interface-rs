use chrome_remote_interface::CdpClient;
use futures::channel::oneshot;
use tokio::net::TcpListener;

#[tokio::test]
async fn connect_ws() -> anyhow::Result<()> {
    pretty_env_logger::init();

    let (tx, rx) = oneshot::channel::<()>();

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await?;
        let stream = tokio_tungstenite::accept_async(stream).await?;
        drop(stream);
        tx.send(()).unwrap();
        anyhow::Result::<_>::Ok(())
    });

    CdpClient::connect_ws(&format!("ws://127.0.0.1:{}", addr.port()).parse()?).await?;
    rx.await?;

    Ok(())
}
