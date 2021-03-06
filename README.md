# chrome-remote-interface

[Chrome DevTools Protocol](https://chromedevtools.github.io/devtools-protocol/) client.

Currently Work In Progress.

### Example

```rust
use chrome_remote_interface::Browser;
use chrome_remote_interface::model::target::{CreateTargetCommand, AttachToTargetCommand};

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let browser = Browser::launcher()
        .headless(true) // headless mode (Default)
        .launch()
        .await?;

    browser.run_with(|mut client| async move {
        // Open new page
        let response = client.request(CreateTargetCommand::builder()
            .url("https://example.org/".into())
            .build()
            .unwrap()
        ).await?;

        // Attach opened page.
        let response = client
            .request(AttachToTargetCommand::new((*response).clone(), Some(true)))
            .await?;

        // construct attached session.
        let mut session = client.session(response);

        // DO STUFF
        // ...

        Ok(())
    }).await
}
```

### License

Licensed under either of
* Apache License, Version 2.0
  ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
* MIT license
  ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)
at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.!

License: MIT OR Apache-2.0
