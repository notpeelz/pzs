#![allow(special_module_name)]

use tracing::error;

mod main;
mod xdg;

fn main() {
  match tokio::runtime::Builder::new_multi_thread()
    .worker_threads(2)
    .max_blocking_threads(20)
    .enable_time()
    .enable_io()
    .build()
  {
    Ok(rt) => {
      rt.block_on(crate::main::main());
      rt.shutdown_background();
    },
    Err(err) => {
      error!("failed to create tokio runtime: {:?}", err);
    },
  }
}
