use ::tracing::debug;
use ls_core::LServer;
use tracing::setup_tracing;

mod tracing;

fn main() {
    let _worker_guard = setup_tracing();
    debug!("================ init ==============");
    let ls = LServer::new();
    ls.run();
}
