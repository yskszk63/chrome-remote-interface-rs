use chrome_remote_interface::model::target::{AttachToTargetCommand, CreateTargetCommand};
use chrome_remote_interface::Browser;

mod common;

#[tokio::test]
async fn test_simple() -> anyhow::Result<()> {
    pretty_env_logger::init();
    common::pgrep_chromium();

    let browser = Browser::launcher()
        .headless(true) // headless mode (Default)
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

            anyhow::Result::<_>::Ok(())
        })
        .await?;

    common::pgrep_chromium();
    Ok(())
}
