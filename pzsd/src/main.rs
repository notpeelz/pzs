use std::future::Future;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::atomic::AtomicBool;
use std::sync::OnceLock;
use std::time::Duration;

use crate::xdg;
use eyre::Context;
use eyre::Result;
use gio::glib;
use gio::prelude::*;
use gtk4::prelude::*;
use gtk4_layer_shell::{Layer, LayerShell};
use notify::Watcher;
use relm4::prelude::*;
use relm4::typed_view::list::RelmListItem;
use relm4::typed_view::list::TypedListView;
use relm4::RelmContainerExt;
use tokio::sync::broadcast;
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::error;

struct AppInit {
  msg_rx: UnboundedReceiver<AppMsg>,
  apps: Vec<xdg::Application>,
}

struct App {
  list_view_wrapper: TypedListView<MyListItem, gtk4::SingleSelection>,
  handle: Option<glib::JoinHandle<()>>,
}

#[derive(Debug)]
enum AppMsg {
  UpdateApps(Vec<xdg::Application>),
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct MyListItem {
  id: String,
  icon: String,
  name: String,
}

// TODO:
// impl MyListItem {
//   fn new(value: u8) -> Self {
//   }
// }

struct MyListItemWidgets {
  image: gtk::Image,
  label: gtk::Label,
}

impl Drop for MyListItemWidgets {
  fn drop(&mut self) {
    dbg!(self.label.label());
  }
}

impl RelmListItem for MyListItem {
  type Root = gtk::Box;
  type Widgets = MyListItemWidgets;

  fn setup(_item: &gtk::ListItem) -> (gtk::Box, MyListItemWidgets) {
    relm4::view! {
      my_box = gtk::Box {
        #[name = "image"]
        gtk::Image,

        #[name = "label"]
        gtk::Label,
      }
    }

    // image.set_icon_size(gtk4::IconSize::Large);
    image.set_pixel_size(64);

    let widgets = MyListItemWidgets { image, label };

    (my_box, widgets)
  }

  fn bind(&mut self, widgets: &mut Self::Widgets, _root: &mut Self::Root) {
    let MyListItemWidgets { image, label } = widgets;

    let icon = gtk4::IconTheme::default().lookup_icon(
      &self.icon,
      &[],
      128,
      1,
      gtk4::TextDirection::Ltr,
      gtk4::IconLookupFlags::empty(),
    );
    image.set_paintable(Some(&icon));
    label.set_label(&self.name);
  }
}

#[allow(dead_code)]
struct AppWidgets {
  my_view: gtk::ListView,
}

impl SimpleComponent for App {
  type Init = AppInit;
  type Input = AppMsg;
  type Output = ();
  type Widgets = AppWidgets;
  type Root = gtk::Window;

  fn init_root() -> Self::Root {
    gtk::Window::builder()
      .title("Simple app")
      .default_width(350)
      .default_height(350)
      .build()
  }

  // Initialize the component.
  fn init(
    init: Self::Init,
    window: Self::Root,
    sender: ComponentSender<Self>,
  ) -> ComponentParts<Self> {
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);

    let vbox = gtk::Box::builder()
      .orientation(gtk::Orientation::Vertical)
      .spacing(5)
      .build();

    let sw = gtk::ScrolledWindow::builder()
      .vexpand(true)
      .build();

    vbox.container_add(&sw);
    window.container_add(&vbox);

    let handle = {
      let mut msg_rx = init.msg_rx;
      relm4::spawn_local(async move {
        loop {
          let Some(msg) = msg_rx.recv().await else {
            break;
          };

          sender.input(msg);
        }
      })
    };

    let mut list_view_wrapper =
      TypedListView::<MyListItem, gtk::SingleSelection>::with_sorting();

    for app in init.apps {
      let no_display = app.desktop_file.no_display.unwrap_or(false);
      if no_display {
        continue;
      }

      let Some(icon) = app.desktop_file.icon else {
        continue;
      };
      let Some(name) = app.desktop_file.name else {
        continue;
      };

      list_view_wrapper.append(MyListItem {
        id: app.id,
        icon,
        name,
      });
    }

    // list_view_wrapper.add_filter(|item| item.value % 2 == 0);
    // list_view_wrapper.set_filter_status(0, false);

    let model = App {
      list_view_wrapper,
      handle: Some(handle),
    };
    let my_view = &model.list_view_wrapper.view;
    sw.container_add(&my_view);

    let widgets = AppWidgets {
      my_view: my_view.clone(),
    };

    ComponentParts { model, widgets }

    // let inc_button = gtk::Button::with_label("Increment");
    // let dec_button = gtk::Button::with_label("Decrement");

    // let label = gtk::Label::new(Some(&format!("Counter: {}", model.counter)));
    // label.set_margin_all(5);

    // window.set_child(Some(&vbox));
    // vbox.set_margin_all(5);
    // vbox.append(&inc_button);
    // vbox.append(&dec_button);
    // vbox.append(&label);

    // inc_button.connect_clicked(clone!(
    //   #[strong]
    //   sender,
    //   move |_| {
    //     sender.input(Msg::Increment);
    //   }
    // ));

    // dec_button.connect_clicked(clone!(
    //   #[strong]
    //   sender,
    //   move |_| {
    //     sender.input(Msg::Decrement);
    //   }
    // ));

    // let widgets = AppWidgets { label };

    // ComponentParts { model, widgets }
  }

  fn shutdown(
    &mut self,
    widgets: &mut Self::Widgets,
    output: relm4::Sender<Self::Output>,
  ) {
    self
      .handle
      .take()
      .unwrap()
      .into_source_id()
      .unwrap()
      .remove();
  }

  fn update(&mut self, msg: Self::Input, _sender: ComponentSender<Self>) {
    match msg {
      AppMsg::UpdateApps(s) => {
        println!("got UpdateApps: {s:#?}");
        // self.counter = self.counter.wrapping_add(1);
      },
    }
  }

  // Update the view to represent the updated model.
  fn update_view(
    &self,
    widgets: &mut Self::Widgets,
    _sender: ComponentSender<Self>,
  ) {
    // widgets
    //   .label
    //   .set_label(&format!("Counter: {}", self.counter));

    // TODO: draw paintable?
    // https://gtk-rs.org/gtk4-rs/git/docs/gtk4/struct.IconTheme.html
  }
}

static SHUTDOWN_TX: OnceLock<broadcast::Sender<()>> = OnceLock::new();

fn shutdown() {
  SHUTDOWN_TX.get().unwrap().send(()).unwrap();
}

#[derive(Default)]
struct TestInterface {}

#[zbus::interface(name = "dev.peelz.pzs.Test")]
impl TestInterface {
  fn shutdown(&mut self) -> zbus::fdo::Result<()> {
    shutdown();
    Ok(())
  }

  async fn play(&mut self) -> zbus::fdo::Result<()> {
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
      .at("/dev/peelz/pzs", TestInterface::default())
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

fn run_gtk(app_init: AppInit) {
  let c = glib::MainContext::default();
  let ml = glib::MainLoop::new(Some(&c), false);

  let ml = Rc::new(ml);
  c.with_thread_default(|| {
    let _ = gio::glib::spawn_future_local({
      let ml = Rc::clone(&ml);
      let mut rx = SHUTDOWN_TX.get().unwrap().subscribe();
      async move {
        rx.recv().await.unwrap();
        println!("bye");
        ml.quit();
      }
    });

    // let b = gio::glib::spawn_future_local({
    //   async move {
    //     tokio::time::sleep(Duration::from_secs(10)).await;
    //     println!("signaling SHUTDOWN_TX");
    //     SHUTDOWN_TX.get().unwrap().send(()).unwrap();
    //   }
    // });

    gtk4::init().expect("failed to initialize gtk");

    let app = RelmApp::new("relm4.example.simple");
    app.run::<App>(app_init);

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

  let (msg_tx, msg_rx) = tokio::sync::mpsc::unbounded_channel::<AppMsg>();

  let apps = xdg::read_applications()
    .await
    .expect("failed to read applications");
  let app_init = AppInit { msg_rx, apps };

  let gtk = tokio::task::spawn_blocking(move || run_gtk(app_init));

  let watcher = tokio::task::spawn(async move {
    let (notify_tx, mut notify_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut watcher = notify::INotifyWatcher::new(
      move |event| {
        let _ = notify_tx.send(event);
      },
      notify::Config::default(),
    )
    .unwrap();

    for d in xdg::BASEDIRS.find_data_files("applications") {
      watcher
        .watch(&*d, notify::RecursiveMode::Recursive)
        .unwrap();
    }

    let dirty = AtomicBool::new(false);
    loop {
      let handle_notify = {
        |x: notify::Result<notify::Event>| match x {
          Ok(ev) if ev.kind.is_access() => {},
          Ok(_) => {
            dirty.store(true, std::sync::atomic::Ordering::SeqCst);
          },
          Err(err) => {
            tracing::warn!(
              "an error occurred while watching applications dir: {err}"
            );
          },
        }
      };

      tokio::select! {
        x = notify_rx.recv() => {
          let Some(r) = x else {
            return;
          };
          handle_notify(r);
          continue;
        },
        _ = tokio::time::sleep(Duration::from_secs(5)) => {
          if !dirty.load(std::sync::atomic::Ordering::SeqCst) {
            continue;
          }
          tracing::info!("timeout after dirty");
        },
      }

      // TODO: deserialize desktop files
      let apps = xdg::read_applications()
        .await
        .expect("failed to read applications");
      msg_tx.send(AppMsg::UpdateApps(apps)).unwrap();
      dirty.store(false, std::sync::atomic::Ordering::SeqCst);
    }
  });

  tokio::select! {
    _ = watcher => {
      println!("watcher died");
    },
    _ = dbus_shutdown => {
      println!("shutting down");
    },
    _ = gtk => {
      println!("gtk rip");
    },
  }

  shutdown();
  drop(dbus_connection);
}
