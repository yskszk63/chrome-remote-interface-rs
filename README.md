# chrome-remote-interface

[Chrome DevTools Protocol](https://chromedevtools.github.io/devtools-protocol/) client.

Currently Work In Progress.

### Supported Browser

- Chromium (latest)

### Browser Discovery

1. Using environemnt variable `CRI_CHROME_BIN` if specified.
2. Search Platform path.
    - Windows: `C:\Program Files\Chromium\Application\chrome.exe`
    - Mac: `/Applications/Chromium.app/Contents/MacOS/Chromium`
    - Linux: `/usr/bin/chromium` or `/usr/bin/chromium-browser`
3. Lookup via `PATH` env var.

### Example

```rust
use chrome_remote_interface::Browser;
use chrome_remote_interface::model::target::{CreateTargetCommand, AttachToTargetCommand};

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    pretty_env_logger::init();
    let browser = Browser::launcher()
        .headless(true) // headless mode (Default)
        .launch()
        .await?;

    let client = browser.connect();
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
