use chrome_remote_interface::model::target::{AttachToTargetCommand, CreateTargetCommand};
use chrome_remote_interface::Browser;

#[tokio::test]
async fn twice() -> anyhow::Result<()> {
    pretty_env_logger::init();

    proc().await?;
    proc().await?;
    Ok(())
}

async fn proc() -> anyhow::Result<()> {
    let browser = Browser::launcher()
        .headless(true) // headless mode (Default)
        .output(true)
        .launch()
        .await?;

    let client = browser.connect();
    // Open new page
    let response = client
        .request(
            CreateTargetCommand::builder()
                .url("https://example.org/".into())
                .build()
                .unwrap(),
        )
        .await?;

    // Attach opened page.
    let response = client
        .request(AttachToTargetCommand::new((*response).clone(), Some(true)))
        .await?;

    // construct attached session.
    let session = client.session(response);

    // DO STUFF
    // ...
    drop(session);
    Ok(())
}
