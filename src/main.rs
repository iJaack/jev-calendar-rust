#[tokio::main]
async fn main() -> anyhow::Result<()> {
    jev_calendar::app::run().await
}
