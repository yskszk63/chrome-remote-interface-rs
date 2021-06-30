use chrome_remote_interface::model::target::{AttachToTargetCommand, CreateTargetCommand};
use chrome_remote_interface::Browser;

mod common;

#[tokio::test]
async fn twice() -> anyhow::Result<()> {
    pretty_env_logger::init();

    common::pgrep_chromium();
    proc().await?;
    common::pgrep_chromium();
    proc().await?;
    common::pgrep_chromium();
    Ok(())
}

async fn proc() -> anyhow::Result<()> {
    let browser = Browser::launcher()
        .headless(true) // headless mode (Default)
        .output(true)
        .launch()
        .await?;

    browser
        .run_with(|mut client| async move {
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
        })
        .await
}
