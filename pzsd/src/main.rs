use std::future::Future;
use std::rc::Rc;
use std::str::FromStr;

use crate::app_launcher::{AppLauncher, AppLauncherInit, AppLauncherMsg};
use eyre::{Context, Result};
use gio::glib;
use gtk4::{self as gtk, prelude::GtkApplicationExt};
use relm4::prelude::*;
use tokio::sync::broadcast;
use tracing::error;

struct Init {
  msg_tx: broadcast::Sender<Msg>,
}

struct Main {}

impl AsyncComponent for Main {
  type Init = Init;
  type Input = Msg;
  type Output = ();
  type Widgets = ();
  type Root = gtk::Window;
  type CommandOutput = ();

  fn init_root() -> Self::Root {
    gtk::Window::builder().visible(false).build()
  }

  async fn init(
    init: Self::Init,
    _root: Self::Root,
    _sender: AsyncComponentSender<Self>,
  ) -> AsyncComponentParts<Self> {
    let app = relm4::main_application();
    let builder = AppLauncher::builder();

    app.add_window(&builder.root);

    builder
      .launch(AppLauncherInit {
        visible: false,
        msg_tx: init.msg_tx.clone(),
        msg_rx: init.msg_tx.subscribe(),
      })
      .detach_runtime();

    AsyncComponentParts {
      model: Main {},
      widgets: (),
    }
  }
}

#[derive(Debug, Clone)]
pub enum Msg {
  Shutdown,
  AppLauncherMsg(AppLauncherMsg),
}

struct DbusInit {
  msg_tx: broadcast::Sender<Msg>,
  msg_rx: broadcast::Receiver<Msg>,
}

struct ShellInterface {
  msg_tx: broadcast::Sender<Msg>,
}

#[zbus::interface(name = "dev.peelz.pzs.Shell")]
impl ShellInterface {
  fn shutdown(&mut self) -> zbus::fdo::Result<()> {
    self.msg_tx.send(Msg::Shutdown).expect("TODO");
    Ok(())
  }
}

struct AppLauncherInterface {
  msg_tx: broadcast::Sender<Msg>,
}

#[zbus::interface(name = "dev.peelz.pzs.AppLauncher")]
impl AppLauncherInterface {
  async fn show(&mut self) -> zbus::fdo::Result<()> {
    self
      .msg_tx
      .send(AppLauncherMsg::SetVisibility(true).into())
      .expect("TODO");
    Ok(())
  }

  async fn hide(&mut self) -> zbus::fdo::Result<()> {
    self
      .msg_tx
      .send(AppLauncherMsg::SetVisibility(false).into())
      .expect("TODO");
    Ok(())
  }

  async fn toggle(&mut self) -> zbus::fdo::Result<()> {
    self
      .msg_tx
      .send(AppLauncherMsg::ToggleVisibility.into())
      .expect("TODO");
    Ok(())
  }
}

async fn setup_dbus(
  dbus_init: DbusInit,
) -> Result<(zbus::Connection, impl Future<Output = ()>)> {
  let DbusInit { mut msg_rx, msg_tx } = dbus_init;

  const DBUS_SESSION_BUS_ADDRESS: &str = "DBUS_SESSION_BUS_ADDRESS";
  let dbus_address = std::env::var(DBUS_SESSION_BUS_ADDRESS).wrap_err(
    format!("{DBUS_SESSION_BUS_ADDRESS} is not set or is invalid"),
  )?;

  const PATH: &str = "/dev/peelz/pzs";

  let connection = zbus::connection::Builder::address(
    zbus::Address::from_str(&dbus_address)
      .wrap_err(format!("failed to parse {DBUS_SESSION_BUS_ADDRESS}"))?,
  )?
  .serve_at(PATH, {
    let msg_tx = msg_tx.clone();
    ShellInterface { msg_tx }
  })
  .wrap_err("failed to register ShellInterface on bus")?
  .serve_at(PATH, {
    let msg_tx = msg_tx.clone();
    AppLauncherInterface { msg_tx }
  })
  .wrap_err("failed to register AppLauncherInterface on bus")?
  .build()
  .await
  .wrap_err("failed to build dbus connection")?;

  connection
    .request_name("dev.peelz.pzs")
    .await
    .wrap_err("failed to register name on bus")?;

  let connection_guard = async move {
    loop {
      match msg_rx.recv().await.expect("TODO") {
        Msg::Shutdown => break,
        _ => (),
      };
    }
  };

  Ok((connection, connection_guard))
}

fn run_gtk(init: Init) {
  let c = glib::MainContext::default();
  let ml = glib::MainLoop::new(Some(&c), false);

  let ml = Rc::new(ml);
  c.with_thread_default(|| {
    let _ = gio::glib::spawn_future_local({
      let ml = Rc::clone(&ml);
      let mut rx = init.msg_tx.subscribe();
      async move {
        loop {
          match rx.recv().await {
            Ok(Msg::Shutdown) | Err(broadcast::error::RecvError::Closed) => {
              println!("bye");
              ml.quit();
              break;
            },
            Ok(_) => {},
            Err(broadcast::error::RecvError::Lagged(_)) => {
              // TODO: log lag?
            },
          };
        }
      }
    });

    gtk::init().expect("failed to initialize gtk");

    let app = RelmApp::new("dev.peelz.pzs").visible_on_activate(false);
    app.allow_multiple_instances(true);
    app.run_async::<Main>(init);
  })
  .unwrap();
}

pub async fn main() {
  let (msg_tx, _) = broadcast::channel::<Msg>(10);

  let (dbus_connection, dbus_shutdown) = match setup_dbus(DbusInit {
    msg_tx: msg_tx.clone(),
    msg_rx: msg_tx.subscribe(),
  })
  .await
  {
    Err(err) => {
      error!({ %err }, "dbus setup failed");
      return;
    },
    Ok(f) => f,
  };
  let gtk = tokio::task::spawn_blocking({
    let msg_tx = msg_tx.clone();
    move || run_gtk(Init { msg_tx })
  });

  let app_launcher = tokio::task::spawn({
    let msg_tx = msg_tx.clone();
    crate::app_launcher::run(msg_tx)
  });

  tokio::select! {
    _ = app_launcher => {
      println!("watcher died");
    },
    _ = dbus_shutdown => {
      println!("shutting down");
    },
    _ = gtk => {
      println!("gtk rip");
    },
  }

  msg_tx.send(Msg::Shutdown).expect("TODO");
  drop(dbus_connection);
}
