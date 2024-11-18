use std::future::Future;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;

use eyre::Context;
use eyre::Result;
use gio::glib;
use gio::prelude::*;
use gtk4::prelude::*;
use gtk4_layer_shell::{Layer, LayerShell};
use notify::Watcher;
use relm4::binding::U8Binding;
use relm4::prelude::*;
use relm4::typed_view::list::RelmListItem;
use relm4::typed_view::list::TypedListView;
use relm4::RelmContainerExt;
use relm4::RelmObjectExt;
use tokio::sync::broadcast;
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::error;

struct AppInit {
  msg_rx: UnboundedReceiver<AppMsg>,
}

struct App {
  list_view_wrapper: TypedListView<MyListItem, gtk4::SingleSelection>,
  handle: Option<glib::JoinHandle<()>>,
}

#[derive(Debug)]
enum AppMsg {
  UpdateApps(String),
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct MyListItem {
  value: u8,
  binding: U8Binding,
}

impl MyListItem {
  fn new(value: u8) -> Self {
    Self {
      value,
      binding: U8Binding::new(0),
    }
  }
}

struct MyListItemWidgets {
  label: gtk::Label,
  label2: gtk::Label,
  button: gtk::CheckButton,
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
            #[name = "label"]
            gtk::Label,

            #[name = "label2"]
            gtk::Label,

            #[name = "button"]
            gtk::CheckButton,
        }
    }

    let widgets = MyListItemWidgets {
      label,
      label2,
      button,
    };

    (my_box, widgets)
  }

  fn bind(&mut self, widgets: &mut Self::Widgets, _root: &mut Self::Root) {
    let MyListItemWidgets {
      label,
      label2,
      button,
    } = widgets;

    label.set_label(&format!("Value: {} ", self.value));
    label2.add_write_only_binding(&self.binding, "label");
    button.set_active(self.value % 2 == 0);
  }
}

#[allow(dead_code)]
struct AppWidgets {
  // window: gtk::Window,
  // vbox: gtk::Box,
  // inc_button: gtk::Button,
  // dec_button: gtk::Button,
  // label: gtk::Label,
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
      .default_width(300)
      .default_height(100)
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

    let list_view_wrapper =
      TypedListView::<MyListItem, gtk::SingleSelection>::with_sorting();

    // list_view_wrapper.add_filter(|item| item.value % 2 == 0);
    // list_view_wrapper.set_filter_status(0, false);

    let model = App {
      list_view_wrapper,
      handle: Some(handle),
    };
    let my_view = &model.list_view_wrapper.view;

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
        println!("got UpdateApps: {s}");
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
  }
}

// https://github.com/wmww/gtk-layer-shell/blob/master/examples/simple-example.c
// fn activate() {
//   // let window = gtk4::Window::new();
//   // window.init_layer_shell();
//   // window.set_layer(Layer::Overlay);

//   // // Set up a widget
//   // // let label = gtk4::Label::new(None);
//   // // let a = gio::glib::markup_escape_text("GTK <a>kek</a>");
//   // // label.set_markup(&format!("<span font_desc=\"20.0\">{}</span>", a));
//   // // label.connect_activate_link(|_, _| {
//   // //   println!("clicked!");
//   // //   gio::glib::Propagation::Stop
//   // // });
//   // // window.set_child(Some(&label));

//   // window.set_visible(true);

//   let app = RelmApp::new("relm4.example.simple");
//   app.run::<App>(0);
// }

pub(crate) static SHUTDOWN_TX: OnceLock<broadcast::Sender<()>> =
  OnceLock::new();

#[derive(Default)]
struct TestInterface {}

#[zbus::interface(name = "dev.peelz.pzs.Test")]
impl TestInterface {
  fn shutdown(&mut self) -> zbus::fdo::Result<()> {
    SHUTDOWN_TX.get().unwrap().send(()).unwrap();
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

fn run_gtk(msg_rx: UnboundedReceiver<AppMsg>) {
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
    app.run::<App>(AppInit { msg_rx });

    ml.run();
  })
  .unwrap();
}

pub(crate) async fn main() {
  let xdg = xdg::BaseDirectories::new().unwrap();
  let (dbus_connection, dbus_shutdown) = match setup_dbus().await {
    Err(err) => {
      error!({ %err }, "dbus setup failed");
      return;
    },
    Ok(f) => f,
  };

  let (msg_tx, msg_rx) = tokio::sync::mpsc::unbounded_channel::<AppMsg>();

  let gtk = tokio::task::spawn_blocking(move || run_gtk(msg_rx));

  tokio::task::spawn(async move {
    let (notify_tx, mut notify_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut watcher = notify::INotifyWatcher::new(
      move |event| {
        let _ = notify_tx.send(event);
      },
      notify::Config::default(),
    )
    .unwrap();

    for d in xdg.find_data_files("applications") {
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

      msg_tx
        .send(AppMsg::UpdateApps("test".into()))
        .unwrap();
      dirty.store(false, std::sync::atomic::Ordering::SeqCst);
    }
  });

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
