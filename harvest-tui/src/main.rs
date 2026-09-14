use std::sync::Arc;
use std::time::Duration;

use clap::Parser;

use harvest_tui::app::App;
use harvest_tui::api::ClientConfig;

#[derive(Parser)]
#[command(name = "harvest-tui", about = "Terminal UI for Harvest")]
struct Args {
    #[arg(long, env = "HARVEST_URL", default_value = "http://localhost:8080")]
    url: String,

    #[arg(long, env = "HARVEST_EMAIL")]
    email: Option<String>,

    #[arg(long, env = "HARVEST_PASSWORD")]
    password: Option<String>,

    #[arg(long, env = "HARVEST_TOKEN")]
    token: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let mut config = ClientConfig {
        base_url: args.url.trim_end_matches('/').to_string(),
        token: args.token,
        cookie_store: None,
    };

    if config.token.is_none() {
        match (args.email.as_ref(), args.password.as_ref()) {
            (Some(email), Some(password)) => {
                let store = Arc::new(reqwest::cookie::Jar::default());
                let client = reqwest::Client::builder()
                    .cookie_provider(store.clone())
                    .timeout(Duration::from_secs(30))
                    .build()?;
                let resp = client
                    .post(format!("{}/auth/login", config.base_url))
                    .json(&serde_json::json!({ "email": email, "password": password }))
                    .send()
                    .await?;
                if !resp.status().is_success() {
                    anyhow::bail!("login failed: {}", resp.status());
                }
                config.cookie_store = Some(store);
            }
            _ => {
                anyhow::bail!(
                    "no credentials: set HARVEST_TOKEN, or HARVEST_EMAIL + HARVEST_PASSWORD"
                );
            }
        }
    }

    let app = App::new(config).await?;
    app.run().await
}
