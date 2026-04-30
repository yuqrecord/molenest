use anyhow::Result;
use tracing_subscriber::{EnvFilter, fmt};

fn main() -> Result<()> {
    init_logging();
    molenest::app::run()
}

fn init_logging() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    fmt().with_env_filter(filter).without_time().init();
}
