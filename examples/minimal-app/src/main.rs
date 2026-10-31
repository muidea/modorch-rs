use modorch::prelude::*;
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let handle = ApplicationBuilder::new("minimal-app")
        .build()?
        .start()
        .await?;
    handle.shutdown_checked(ShutdownOptions::graceful()).await?;
    Ok(())
}
