use std::rc::Rc;

use crate::{
  app_launcher::{AppLauncher, AppLauncherInit},
  dbus::{Messenger, Msg},
};
use gio::glib;
use gtk4::{self as gtk, prelude::GtkApplicationExt};
use relm4::prelude::*;
use tokio::sync::broadcast;
use tracing::{debug, error, warn};

struct Init {
  messenger: Messenger,
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
        messenger: init.messenger,
      })
      .detach_runtime();

    AsyncComponentParts {
      model: Main {},
      widgets: (),
    }
  }
}

pub struct ShellInterface {
  pub messenger: Messenger,
}

#[zbus::interface(name = "dev.peelz.pzs.Shell")]
impl ShellInterface {
  fn shutdown(&mut self) -> zbus::fdo::Result<()> {
    self.messenger.send_msg(Msg::Shutdown)
  }
}

fn run_gtk(init: Init) {
  let c = glib::MainContext::default();
  let ml = glib::MainLoop::new(Some(&c), false);

  let ml = Rc::new(ml);
  c.with_thread_default(|| {
    let _ = gio::glib::spawn_future_local({
      let ml = Rc::clone(&ml);
      let mut messenger = init.messenger.clone();
      async move {
        loop {
          match messenger.rx().recv().await {
            Ok(Msg::Shutdown) | Err(broadcast::error::RecvError::Closed) => {
              ml.quit();
              break;
            },
            Ok(_) => {},
            Err(broadcast::error::RecvError::Lagged(_)) => {
              warn!("gtk receiver lagged");
            },
          };
        }
      }
    });

    gtk::init().expect("failed to initialize gtk");

    let app = RelmApp::new("dev.peelz.pzs").visible_on_activate(false);
    app.allow_multiple_instances(true);

    relm4::set_global_css(include_str!("app_launcher.css"));

    app.run_async::<Main>(init);
  })
  .unwrap();
}

pub async fn main() {
  let mut messenger = Messenger::new(10);

  let (dbus_connection, dbus_shutdown) =
    match crate::dbus::setup(messenger.clone()).await {
      Err(err) => {
        error!({ %err }, "dbus setup failed");
        return;
      },
      Ok(f) => f,
    };
  let gtk = tokio::task::spawn_blocking({
    let messenger = messenger.clone();
    move || run_gtk(Init { messenger })
  });

  let app_launcher =
    tokio::task::spawn(crate::app_launcher::run(messenger.clone()));

  tokio::select! {
    _ = app_launcher => {
      panic!("app_launcher died");
    },
    _ = dbus_shutdown => {
      debug!("shutdown via dbus");
    },
    _ = gtk => {
      panic!("gtk died");
    },
  }

  messenger.tx().send(Msg::Shutdown).unwrap();
  drop(dbus_connection);
}
