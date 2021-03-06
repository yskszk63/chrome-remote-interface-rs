use chrome_remote_interface::Browser;

#[tokio::test]
async fn connect_ws() -> anyhow::Result<()> {
    pretty_env_logger::init();

    let browser = Browser::launcher()
        .output(true)
        .use_pipe(false)
        .launch()
        .await?;

    browser.run_with(|_| async { Ok(()) }).await
}
