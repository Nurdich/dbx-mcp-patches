use std::sync::Arc;

use dbx_mcp::{DbxBackend, DbxMcpServer, LocalBackend, WebBackend};
use rmcp::ServiceExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let backend: Arc<dyn DbxBackend> = if let Ok(base_url) = std::env::var("DBX_WEB_URL") {
        let password = std::env::var("DBX_WEB_PASSWORD").unwrap_or_default();
        let web = WebBackend::new(base_url, password).map_err(std::io::Error::other)?;
        // MCP hosts often hide env issues; log mode to stderr (never log the password).
        eprintln!(
            "[dbx-mcp] web mode enabled: url={} password_configured={}",
            web.base_url(),
            web.password_configured()
        );
        Arc::new(web)
    } else {
        let db_path = dbx_mcp::paths::storage_db_path().map_err(std::io::Error::other)?;
        eprintln!("[dbx-mcp] local mode: storage={}", db_path.display());
        Arc::new(LocalBackend::open(&db_path).await.map_err(std::io::Error::other)?)
    };
    let service = DbxMcpServer::new(backend).serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}
