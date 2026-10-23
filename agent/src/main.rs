//! `clockwork-mcp`: Clockwork's extraction tools as an MCP server.
//!
//! Serves stdio by default. Pass `--http <IP:PORT>` to serve streamable HTTP
//! at `/mcp` instead, e.g. `clockwork-mcp --http 127.0.0.1:3128`.

use std::sync::Arc;

use clockwork_agent::{presence::Presence, server::Clockwork};

use rmcp::{
    ServiceExt,
    transport::{
        stdio,
        streamable_http_server::{
            StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
        },
    },
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let http_addr = match (args.next().as_deref(), args.next()) {
        (Some("--http"), Some(addr)) => Some(addr),
        (Some("--http"), None) => {
            return Err("--http needs an address, e.g. --http 127.0.0.1:3128".into());
        }
        (Some(other), _) => return Err(format!("unknown argument: {other}").into()),
        (None, _) => None,
    };

    match http_addr {
        Some(addr) => {
            let presence = Presence::start(format!("http {addr}"));
            let result = serve_http(&addr, presence.clone()).await;
            presence.stop();
            result
        }
        None => {
            let presence = Presence::start("stdio");
            let server = Clockwork::new().with_presence(presence.clone());
            let result: Result<(), Box<dyn std::error::Error>> = async {
                server.serve(stdio()).await?.waiting().await?;
                Ok(())
            }
            .await;
            presence.stop();
            result
        }
    }
}

async fn serve_http(addr: &str, presence: Presence) -> Result<(), Box<dyn std::error::Error>> {
    // One Library for every session: loading it reads and hashes every ROM.
    let workspace = Clockwork::load_workspace();
    let service = StreamableHttpService::new(
        move || Ok(Clockwork::with_workspace(workspace.clone()).with_presence(presence.clone())),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );
    let app = axum::Router::new().route_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("clockwork-mcp serving streamable HTTP at http://{addr}/mcp");
    if listener.local_addr().is_ok_and(|a| !a.ip().is_loopback()) {
        // rmcp only answers requests whose Host is localhost or a loopback address (against
        // DNS rebinding), so other machines can't use the server even on a public address.
        eprintln!(
            "note: only requests addressed to localhost are answered; \
             this server has no authentication, so it is not opened to other machines"
        );
    }
    axum::serve(listener, app).await?;
    Ok(())
}
