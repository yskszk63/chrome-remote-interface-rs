use chrome_remote_interface::model::page::{self, CaptureScreenshotCommand, NavigateCommand};
use chrome_remote_interface::model::runtime::EvaluateCommand;
use chrome_remote_interface::model::target::{AttachToTargetCommand, CreateTargetCommand};
use chrome_remote_interface::model::Event;

use chrome_remote_interface::Browser;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    pretty_env_logger::init();

    let browser = Browser::launcher()
        .headless(false)
        .output(true)
        .launch()
        .await?;
    let client = browser.connect().await?;

    let mut events = client.events().await;
    tokio::spawn(async move {
        while let Some(evt) = events.next().await {
            println!("{:?}", evt);
        }
    });

    let response = client
        .request(
            CreateTargetCommand::builder()
                .url("http://example.org/".into())
                .build()
                .unwrap(),
        )
        .await?;
    println!("{:?}", response);

    let response = client
        .request(AttachToTargetCommand::new((*response).clone(), Some(true)))
        .await?;
    println!("{:?}", response);

    let session = client.session(response);
    let mut events = session.events().await;
    tokio::spawn(async move {
        while let Some(evt) = events.next().await {
            println!("{:?}", evt);
        }
    });
    drop(client);

    let mut events = session.events().await;

    let response = session.request(page::EnableCommand::new()).await?;
    println!("{:?}", response);

    while let Some(evt) = events.next().await {
        if let Event::PageLoadEventFired(..) = evt {
            break;
        }
    }

    let script = r#"document.querySelector('p').textContent = "Hello, World!""#.into();
    let response = session
        .request(
            EvaluateCommand::builder()
                .expression(script)
                .build()
                .unwrap(),
        )
        .await?;
    println!("{:?}", response);

    let response = session
        .request(CaptureScreenshotCommand::builder().build().unwrap())
        .await?;
    let data = response.data();
    let dataurl = format!("data:image/png;base64,{}", data);

    let response = session
        .request(NavigateCommand::builder().url(dataurl).build().unwrap())
        .await?;
    println!("{:?}", response);

    let mut events = session.events().await;
    while let Some(event) = events.next().await {
        println!("** {:?}", event);
    }

    Ok(())
}
