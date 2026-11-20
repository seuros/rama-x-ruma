//! Ask a homeserver who an access token belongs to.
//!
//! ```sh
//! cargo run -p rama-x-ruma --example whoami          # in one shell
//! cargo run -p rama-x-ruma --features client --example whoami_client
//! cargo run -p rama-x-ruma --features client --example whoami_client -- http://127.0.0.1:8008 wrong
//! ```
//!
//! Arguments: homeserver URL (default `http://127.0.0.1:8008`) and access
//! token (default `secret`, which the `whoami` example accepts). HTTPS
//! homeservers need rama's `rustls` or `boring` feature.

use std::time::Duration;

use rama::http::client::DefaultHttpWebClient;
use rama_x_ruma::MatrixClient;
use ruma::api::client::account::whoami::v3 as whoami;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut args = std::env::args().skip(1);
    let homeserver = args
        .next()
        .unwrap_or_else(|| "http://127.0.0.1:8008".to_owned());
    let token = args.next().unwrap_or_else(|| "secret".to_owned());

    let client = MatrixClient::new(DefaultHttpWebClient::default(), homeserver)
        .with_access_token(token)
        .with_timeout(Duration::from_secs(30));

    match client.send(whoami::Request::new()).await {
        Ok(me) => {
            let device = me.device_id.as_ref().map_or("-", |d| d.as_str());
            println!("{} (device {device})", me.user_id);
            Ok(())
        }
        Err(error) => {
            // errcode and retry_after come straight from ruma's error type
            if let Some(errcode) = error.errcode() {
                eprintln!("homeserver said {errcode}: {error}");
            }
            if let Some(wait) = error.retry_after() {
                eprintln!("rate limited; retry in {wait:?}");
            }
            Err(error.into())
        }
    }
}
