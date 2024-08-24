use std::future::Future;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::OnceLock;
use std::time::Duration;

use eyre::Context;
use eyre::Result;
use gio::glib;
use gio::prelude::*;
use gtk4::prelude::*;
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use tokio::sync::broadcast;
use tracing::error;

// https://github.com/wmww/gtk-layer-shell/blob/master/examples/simple-example.c
fn activate() {
  // Create a normal GTK window however you like
  let window = gtk4::Window::new();

  // Before the window is first realized, set it up to be a layer surface
  window.init_layer_shell();

  // Display above normal windows
  window.set_layer(Layer::Overlay);

  // Push other windows out of the way
  window.auto_exclusive_zone_enable();

  // The margins are the gaps around the window's edges
  // Margins and anchors can be set like this...
  // window.set_margin(Edge::Left, 40);
  // window.set_margin(Edge::Right, 40);
  // window.set_margin(Edge::Top, 20);

  // ... or like this
  // Anchors are if the window is pinned to each edge of the output
  let anchors = [
    (Edge::Left, true),
    (Edge::Right, true),
    (Edge::Top, false),
    (Edge::Bottom, true),
  ];

  for (anchor, state) in anchors {
    window.set_anchor(anchor, state);
  }

  // Set up a widget
  let label = gtk4::Label::new(None);
  let a = gio::glib::markup_escape_text("GTK <a>kek</a>");
  label.set_markup(&format!("<span font_desc=\"20.0\">{}</span>", a));
  label.connect_activate_link(|_, _| {
    println!("clicked!");
    gio::glib::Propagation::Stop
  });
  window.set_child(Some(&label));
  window.show()
}

pub(crate) static SHUTDOWN_TX: OnceLock<broadcast::Sender<()>> =
  OnceLock::new();

#[derive(Default)]
struct UnredirectInterface {}

#[zbus::interface(name = "dev.peelz.pzs.Unredirect")]
impl UnredirectInterface {
  fn blah(&mut self) -> zbus::fdo::Result<()> {
    SHUTDOWN_TX.get().unwrap().send(()).unwrap();
    Ok(())
  }
}

async fn setup_dbus() -> Result<(zbus::Connection, impl Future<Output = ()>)> {
  const DBUS_SESSION_BUS_ADDRESS: &str = "DBUS_SESSION_BUS_ADDRESS";
  let dbus_address = std::env::var(DBUS_SESSION_BUS_ADDRESS).wrap_err(
    format!("{DBUS_SESSION_BUS_ADDRESS} is not set or is invalid"),
  )?;

  let connection = zbus::connection::Builder::address(
    zbus::Address::from_str(&dbus_address)
      .wrap_err(format!("failed to parse {DBUS_SESSION_BUS_ADDRESS}"))?,
  )?
  .build()
  .await
  .wrap_err("failed to build dbus connection")?;

  {
    let object_server = connection.object_server();

    object_server
      .at("/dev/peelz/pzs", UnredirectInterface::default())
      .await
      .wrap_err("failed to register bus interface")
      .and_then(|x| {
        if x {
          Ok(())
        } else {
          eyre::bail!("dbus interface is already registered")
        }
      })?;

    connection
      .request_name("dev.peelz.pzs")
      .await
      .wrap_err("failed to register name on bus")?;
  }

  let f = {
    let (tx, mut rx) = broadcast::channel::<()>(1);
    SHUTDOWN_TX.set(tx).unwrap();
    async move {
      rx.recv().await.unwrap();
    }
  };

  Ok((connection, f))
}

fn run_gtk() {
  let c = glib::MainContext::default();
  let ml = glib::MainLoop::new(Some(&c), false);

  let ml = Rc::new(ml);
  c.with_thread_default(|| {
    let a = gio::glib::spawn_future_local({
      let ml = Rc::clone(&ml);
      let mut rx = SHUTDOWN_TX.get().unwrap().subscribe();
      async move {
        rx.recv().await.unwrap();
        println!("bye");
        ml.quit();
      }
    });

    let b = gio::glib::spawn_future_local({
      async move {
        tokio::time::sleep(Duration::from_secs(10)).await;
        println!("signaling SHUTDOWN_TX");
        SHUTDOWN_TX.get().unwrap().send(()).unwrap();
      }
    });

    gtk4::init().expect("failed to initialize gtk");

    activate();

    ml.run();
  })
  .unwrap();
}

pub(crate) async fn main() {
  let (dbus_connection, dbus_shutdown) = match setup_dbus().await {
    Err(err) => {
      error!({ %err }, "dbus setup failed");
      return;
    },
    Ok(f) => f,
  };

  let gtk = tokio::task::spawn_blocking(run_gtk);

  tokio::select! {
    _ = dbus_shutdown => {
      println!("shutting down");
    },
    _ = gtk => {
      println!("gtk rip");
    },
  }

  drop(dbus_connection);
}
