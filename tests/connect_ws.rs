use chrome_remote_interface::Browser;

#[tokio::test]
async fn connect_ws() -> anyhow::Result<()> {
    pretty_env_logger::init();

    let browser = Browser::launcher().use_pipe(false).launch().await?;

    let (_, task) = browser.connect().await?;
    tokio::spawn(task);

    Ok(())
}
