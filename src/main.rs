use caldav_mcp::{accounts::Settings, application::Application, server::Server};
use clap::Parser;
use rmcp::ServiceExt;
#[derive(Parser)]
#[command(version, about = "Stateless multi-account CalDAV MCP server (stdio)")]
struct Arguments {
    #[arg(long)]
    config: std::path::PathBuf,
}
#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("caldav-mcp: {e}");
        std::process::exit(1);
    }
}
async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = Arguments::parse();
    let source = std::fs::read_to_string(args.config)?;
    let settings: Settings = toml::from_str(&source)?;
    let server = Server::new(Application::new(settings)?);
    server
        .serve(rmcp::transport::stdio())
        .await?
        .waiting()
        .await?;
    Ok(())
}
