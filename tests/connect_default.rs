use chrome_remote_interface::Browser;

#[tokio::test]
async fn connect_default() -> anyhow::Result<()> {
    pretty_env_logger::init();

    let browser = Browser::launcher()
        .output(true)
        .launch()
        .await?;

    browser.run_with(|_| async { Ok(()) }).await
}
