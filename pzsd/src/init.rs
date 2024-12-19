#![allow(special_module_name)]

mod app_launcher;
mod dbus;
mod list;
mod main;
mod xdg;

pub static XDG_CURRENT_DESKTOP: once_cell::sync::Lazy<Option<Vec<String>>> =
  once_cell::sync::Lazy::<Option<Vec<String>>>::new(|| {
    std::env::var("XDG_CURRENT_DESKTOP")
      .ok()
      .map(|x| {
        x.split(':')
          .map(ToString::to_string)
          .collect::<Vec<_>>()
      })
  });

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
      panic!("failed to create tokio runtime: {:?}", err);
    },
  }
}
