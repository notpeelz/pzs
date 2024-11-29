use std::future::Future;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use crate::xdg;
use eyre::{Context, Result};
use gio::glib;
use gtk::gdk;
use gtk::prelude::WidgetExt;
use gtk::IconPaintable;
use gtk4::prelude::EventControllerExt;
use gtk4_layer_shell::{Layer, LayerShell};
use notify::Watcher;
use relm4::prelude::*;
use relm4::typed_view::list::RelmListItem;
use relm4::typed_view::list::TypedListView;
use relm4::RelmContainerExt;
use tokio::sync::broadcast;
use tracing::error;

struct DbusInit {
  msg_tx: broadcast::Sender<AppMsg>,
  msg_rx: broadcast::Receiver<AppMsg>,
}

struct AppLauncherInit {
  visible: bool,
  msg_tx: broadcast::Sender<AppMsg>,
  msg_rx: broadcast::Receiver<AppMsg>,
  apps: Vec<xdg::Application>,
}

struct AppLauncher {
  visible: bool,
  new_apps: Option<Vec<xdg::Application>>,
  input_buffer: gtk::EntryBuffer,
  list_view_wrapper: TypedListView<MyListItem, gtk::SingleSelection>,
  handle: Option<glib::JoinHandle<()>>,
}

#[derive(Debug, Clone)]
enum AppMsg {
  Shutdown,
  UpdateApps(Vec<xdg::Application>),
  SetVisibility(bool),
  ToggleVisibility,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct MyListItem {
  id: String,
  icon: String,
  name: String,
}

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

    image.set_pixel_size(48);

    let widgets = MyListItemWidgets { image, label };

    (my_box, widgets)
  }

  fn bind(&mut self, widgets: &mut Self::Widgets, _root: &mut Self::Root) {
    let MyListItemWidgets { image, label } = widgets;

    let icon = if self.icon.starts_with('/') {
      IconPaintable::for_file(&gio::File::for_path(&self.icon), 48, 1)
    } else {
      let theme =
        gtk::IconTheme::for_display(&gdk::Display::default().unwrap());

      if theme.has_icon(&self.icon) {
        theme.lookup_icon(
          &self.icon,
          &[],
          48,
          1,
          gtk::TextDirection::None,
          gtk::IconLookupFlags::empty(),
        )
      } else {
        theme.lookup_icon(
          "application-x-executable",
          &[],
          48,
          1,
          gtk::TextDirection::None,
          gtk::IconLookupFlags::empty(),
        )
      }
    };

    image.set_paintable(Some(&icon));
    label.set_label(&self.name);
  }
}

#[allow(dead_code)]
struct AppLauncherWidgets {
  list_view: gtk::ListView,
  window: gtk::Window,
}

impl AppLauncher {
  fn set_visibility(&mut self, visible: bool) {
    self.visible = visible;

    if visible {
      if let Some(new_apps) = self.new_apps.take() {
        self.new_apps = None;
        update_list_view(&mut self.list_view_wrapper, &new_apps);
      }
    }
  }
}

fn update_list_view(
  list_view: &mut TypedListView<MyListItem, gtk::SingleSelection>,
  apps: &[xdg::Application],
) {
  list_view.clear();
  for app in apps.into_iter() {
    let desktop_file = &app.desktop_file;
    let no_display = desktop_file.no_display.unwrap_or(false);
    if no_display {
      continue;
    }

    let Some(ref icon) = desktop_file.icon else {
      continue;
    };
    let Some(ref name) = desktop_file.name else {
      continue;
    };

    list_view.append(MyListItem {
      id: app.id.clone(),
      icon: icon.clone(),
      name: name.clone(),
    });
  }
}

impl SimpleComponent for AppLauncher {
  type Init = AppLauncherInit;
  type Input = AppMsg;
  type Output = ();
  type Widgets = AppLauncherWidgets;
  type Root = gtk::Window;

  fn init_root() -> Self::Root {
    gtk::Window::builder()
      .title("Simple app")
      .default_width(350)
      .default_height(350)
      .visible(false)
      .build()
  }

  fn init(
    init: Self::Init,
    window: Self::Root,
    sender: ComponentSender<Self>,
  ) -> ComponentParts<Self> {
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_visible(init.visible);
    window.set_keyboard_mode(gtk4_layer_shell::KeyboardMode::OnDemand);

    let vbox = gtk::Box::builder()
      .orientation(gtk::Orientation::Vertical)
      .spacing(5)
      .build();

    let input_buffer = gtk::EntryBuffer::new(glib::GStr::NONE);
    let input = gtk::Entry::builder()
      .buffer(&input_buffer)
      .margin_top(10)
      .placeholder_text("Search")
      .build();
    let input_key_controller = gtk::EventControllerKey::new();
    input.add_controller(input_key_controller.clone());

    window.add_controller({
      let key_controller = gtk::EventControllerKey::new();
      let sender = sender.clone();
      key_controller.connect_key_pressed(move |_, keyval, _, _| {
        if keyval == gdk::Key::Escape {
          sender.input(AppMsg::SetVisibility(false));
          return glib::Propagation::Stop;
        }

        glib::Propagation::Proceed
      });
      key_controller.set_propagation_phase(gtk::PropagationPhase::Capture);
      key_controller
    });

    let sw = gtk::ScrolledWindow::builder()
      .vexpand(true)
      .can_focus(false)
      .build();

    vbox.container_add(&input);
    vbox.container_add(&sw);
    window.container_add(&vbox);

    let handle = {
      let sender = sender.clone();
      let mut msg_rx = init.msg_rx;
      relm4::spawn_local(async move {
        loop {
          let msg = match msg_rx.recv().await {
            Ok(msg) => msg,
            Err(broadcast::error::RecvError::Closed) => {
              break;
            },
            Err(broadcast::error::RecvError::Lagged(_)) => {
              // TODO: log lag?
              continue;
            },
          };

          sender.input(msg);
        }
      })
    };

    let mut list_view_wrapper =
      TypedListView::<MyListItem, gtk::SingleSelection>::with_sorting();
    let list_view = list_view_wrapper.view.clone();
    sw.container_add(&list_view);

    // list_view_wrapper.add_filter(|item| item.value % 2 == 0);
    // list_view_wrapper.set_filter_status(0, false);

    update_list_view(&mut list_view_wrapper, &init.apps);

    let model = AppLauncher {
      visible: init.visible,
      new_apps: None,
      input_buffer,
      list_view_wrapper,
      handle: Some(handle),
    };

    let widgets = AppLauncherWidgets {
      list_view: list_view.clone(),
      window,
    };

    ComponentParts { model, widgets }
  }

  fn shutdown(
    &mut self,
    _widgets: &mut Self::Widgets,
    _output: relm4::Sender<Self::Output>,
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
      AppMsg::Shutdown => {},
      AppMsg::UpdateApps(new_apps) => {
        self.new_apps = Some(new_apps);
      },
      AppMsg::SetVisibility(visible) => {
        self.set_visibility(visible);
      },
      AppMsg::ToggleVisibility => {
        self.set_visibility(!self.visible);
      },
    }
  }

  // Update the view to represent the updated model.
  fn update_view(
    &self,
    widgets: &mut Self::Widgets,
    _sender: ComponentSender<Self>,
  ) {
    widgets.window.set_visible(self.visible);
  }
}

struct TestInterface {
  msg_tx: broadcast::Sender<AppMsg>,
}

#[zbus::interface(name = "dev.peelz.pzs.Test")]
impl TestInterface {
  fn shutdown(&mut self) -> zbus::fdo::Result<()> {
    self.msg_tx.send(AppMsg::Shutdown).expect("TODO");
    Ok(())
  }

  async fn show(&mut self) -> zbus::fdo::Result<()> {
    self
      .msg_tx
      .send(AppMsg::SetVisibility(true))
      .expect("TODO");
    Ok(())
  }

  async fn hide(&mut self) -> zbus::fdo::Result<()> {
    self
      .msg_tx
      .send(AppMsg::SetVisibility(false))
      .expect("TODO");
    Ok(())
  }

  async fn toggle(&mut self) -> zbus::fdo::Result<()> {
    self
      .msg_tx
      .send(AppMsg::ToggleVisibility)
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
      .at("/dev/peelz/pzs", TestInterface { msg_tx })
      .await
      .wrap_err("failed to register bus interface")
      .and_then(|x| {
        if !x {
          eyre::bail!("dbus interface is already registered")
        }

        Ok(())
      })?;

    connection
      .request_name("dev.peelz.pzs")
      .await
      .wrap_err("failed to register name on bus")?;
  }

  let connection_guard = async move {
    loop {
      match msg_rx.recv().await.expect("TODO") {
        AppMsg::Shutdown => break,
        _ => (),
      };
    }
  };

  Ok((connection, connection_guard))
}

fn run_gtk(app_init: AppLauncherInit) {
  let c = glib::MainContext::default();
  let ml = glib::MainLoop::new(Some(&c), false);

  let ml = Rc::new(ml);
  c.with_thread_default(|| {
    let _ = gio::glib::spawn_future_local({
      let ml = Rc::clone(&ml);
      let mut rx = app_init.msg_tx.subscribe();
      async move {
        loop {
          match rx.recv().await {
            Ok(AppMsg::Shutdown) | Err(broadcast::error::RecvError::Closed) => {
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
    app.run::<AppLauncher>(app_init);
  })
  .unwrap();
}

pub(crate) async fn main() {
  let (msg_tx, _) = broadcast::channel::<AppMsg>(10);

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

  let apps = xdg::read_applications()
    .await
    .expect("failed to read applications");
  let app_init = AppLauncherInit {
    visible: false,
    msg_tx: msg_tx.clone(),
    msg_rx: msg_tx.subscribe(),
    apps,
  };

  let gtk = tokio::task::spawn_blocking(move || run_gtk(app_init));

  let watcher = tokio::task::spawn({
    let msg_tx = msg_tx.clone();
    async move {
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

        let apps = xdg::read_applications()
          .await
          .expect("failed to read applications");
        msg_tx.send(AppMsg::UpdateApps(apps)).unwrap();
        dirty.store(false, std::sync::atomic::Ordering::SeqCst);
      }
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

  msg_tx.send(AppMsg::Shutdown).expect("TODO");
  drop(dbus_connection);
}
