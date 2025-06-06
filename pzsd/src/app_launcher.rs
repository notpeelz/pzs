use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use crate::{
  dbus::{Messenger, Msg},
  list::{RelmListItem, TypedListView},
  shell::shell_parse_args,
  xdg, XDG_CURRENT_DESKTOP,
};
use eyre::{bail, Result};
use gio::{
  glib,
  prelude::{ListModelExt, ObjectExt},
};
use gtk4::{
  self as gtk,
  prelude::{EditableExt, EventControllerExt, GtkWindowExt},
  IconPaintable,
};
use gtk4::{gdk, prelude::WidgetExt};
use gtk4_layer_shell::{Layer, LayerShell};
use notify::Watcher;
use relm4::{
  prelude::{AsyncComponent, AsyncComponentParts},
  AsyncComponentSender, RelmContainerExt,
};
use tokio::sync::broadcast;
use tracing::{debug, warn};

pub struct AppLauncherInit {
  pub visible: bool,
  pub dbus_connection: zbus::Connection,
  pub messenger: Messenger,
}

pub struct AppLauncher {
  visible: bool,
  dbus_connection: zbus::Connection,
  new_apps: Option<Vec<xdg::Application>>,
  list_view_wrapper: TypedListView<AppItem, gtk::SingleSelection>,
  input: gtk::SearchEntry,
  handle: Option<glib::JoinHandle<()>>,
}

pub struct AppLauncherInterface {
  pub messenger: Messenger,
}

#[zbus::interface(name = "dev.peelz.pzs.AppLauncher")]
impl AppLauncherInterface {
  fn show(&mut self) -> zbus::fdo::Result<()> {
    self
      .messenger
      .send_msg(AppLauncherMsg::SetVisibility(true))
  }

  fn hide(&mut self) -> zbus::fdo::Result<()> {
    self
      .messenger
      .send_msg(AppLauncherMsg::SetVisibility(false))
  }

  fn toggle(&mut self) -> zbus::fdo::Result<()> {
    self
      .messenger
      .send_msg(AppLauncherMsg::ToggleVisibility)
  }
}

#[derive(Debug, Clone)]
pub enum AppLauncherMsg {
  UpdateApps(Vec<xdg::Application>),
  UpdateFilter(String),
  SetVisibility(bool),
  ToggleVisibility,
  Run,
}

impl Into<Msg> for AppLauncherMsg {
  fn into(self) -> Msg {
    Msg::AppLauncherMsg(self)
  }
}

#[derive(Debug, PartialEq, Eq)]
struct AppItem(pub xdg::Application);

struct AppItemWidgets {
  image: gtk::Image,
  label: gtk::Label,
}

impl RelmListItem for AppItem {
  type Root = gtk::Box;
  type Widgets = AppItemWidgets;

  fn setup(_item: &gtk::ListItem) -> (gtk::Box, AppItemWidgets) {
    relm4::view! {
      my_box = gtk::Box {
        #[name = "image"]
        gtk::Image {
          add_css_class: "app-icon",
          set_pixel_size: 48,
        },

        #[name = "label"]
        gtk::Label {
          add_css_class: "app-label",
        }
      }
    }

    let widgets = AppItemWidgets { image, label };

    (my_box, widgets)
  }

  fn bind(&mut self, widgets: &mut Self::Widgets, _root: &mut Self::Root) {
    let AppItemWidgets { image, label } = widgets;

    if let Some(name) = &self.0.name {
      label.set_label(&name);
    }

    if let Some(icon) = &self.0.icon {
      let icon = if icon.starts_with('/') {
        IconPaintable::for_file(&gio::File::for_path(&icon), 48, 1)
      } else {
        let theme =
          gtk::IconTheme::for_display(&gdk::Display::default().unwrap());

        if theme.has_icon(&icon) {
          theme.lookup_icon(
            &icon,
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
    }
  }
}

#[allow(dead_code)]
pub struct AppLauncherWidgets {
  window: gtk::Window,
}

impl AppLauncher {
  fn set_visibility(&mut self, visible: bool) {
    self.visible = visible;

    if visible {
      if let Some(new_apps) = self.new_apps.take() {
        self.new_apps = None;
        update_list_view(&mut self.list_view_wrapper, new_apps);
      }
    }
  }

  fn select(&self, position: u32) {
    if position >= self.list_view_wrapper.selection_model.n_items() {
      return;
    }

    self
      .list_view_wrapper
      .selection_model
      .set_selected(position);

    self.list_view_wrapper.view.scroll_to(
      position,
      gtk::ListScrollFlags::FOCUS,
      None,
    );
  }
}

fn update_list_view(
  list_view: &mut TypedListView<AppItem, gtk::SingleSelection>,
  apps: Vec<xdg::Application>,
) {
  list_view.clear();
  for app in apps.into_iter() {
    let desktop_file = &app;

    let no_display = desktop_file.no_display.unwrap_or(false);
    if no_display {
      continue;
    }

    if let Some(only_show_in) = &desktop_file.only_show_in {
      if let Some(desktops) = &*XDG_CURRENT_DESKTOP {
        for desktop in desktops {
          if !only_show_in.contains(desktop) {
            continue;
          }
        }
      }
    }

    list_view.append(AppItem(app));
  }
}

impl AppLauncher {
  async fn launch_app(&self, app: &AppItem) -> Result<()> {
    let mut workdir = app.0.path.as_ref().map(PathBuf::from);
    if workdir
      .as_ref()
      .is_some_and(|x| x.as_os_str().is_empty())
    {
      workdir = None;
    }
    if workdir.as_ref().is_some_and(|x| !x.is_dir()) {
      bail!("the Path property isn't a valid directory");
    }
    let workdir = workdir.map_or_else(std::env::current_dir, |x| Ok(x))?;

    let exec = app.0.expand_exec(&[])?;
    let (exe, args) = {
      let mut v = shell_parse_args(&exec);
      if v.is_empty() {
        bail!("the Exec property can't be empty");
      }

      let args = v.split_off(1);
      (v[0].clone(), args)
    };

    debug!(?exec, ?workdir, ?exe, ?args, "launching app");

    crate::spawning::spawn(workdir, exe, args);

    Ok(())
  }
}

impl AsyncComponent for AppLauncher {
  type Init = AppLauncherInit;
  type Input = Msg;
  type Output = ();
  type Widgets = AppLauncherWidgets;
  type Root = gtk::Window;
  type CommandOutput = ();

  fn init_root() -> Self::Root {
    gtk::Window::builder()
      .title("App Launcher")
      .default_width(350)
      .default_height(350)
      .visible(false)
      .build()
  }

  async fn init(
    init: Self::Init,
    window: Self::Root,
    sender: AsyncComponentSender<Self>,
  ) -> AsyncComponentParts<Self> {
    let apps = xdg::application::read_applications()
      .await
      .expect("failed to read applications");

    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_visible(init.visible);
    window.set_keyboard_mode(gtk4_layer_shell::KeyboardMode::OnDemand);
    window.add_css_class("app-launcher");

    window.connect_notify_local(Some("is-active"), {
      let sender = sender.clone();
      move |win, _| {
        if !win.is_active() {
          sender.input(AppLauncherMsg::SetVisibility(false).into());
        }
      }
    });

    let vbox = gtk::Box::builder()
      .orientation(gtk::Orientation::Vertical)
      .spacing(5)
      .build();

    let input = gtk::SearchEntry::builder()
      .margin_top(10)
      .margin_start(10)
      .margin_end(10)
      .search_delay(0)
      .placeholder_text("Search")
      .css_classes(["search"])
      .build();

    let sw = gtk::ScrolledWindow::builder()
      .vexpand(true)
      .can_focus(false)
      .build();

    vbox.container_add(&input);
    vbox.container_add(&sw);
    window.container_add(&vbox);

    let handle = {
      let sender = sender.clone();
      let mut messenger = init.messenger;
      relm4::spawn_local(async move {
        loop {
          let msg = match messenger.rx().recv().await {
            Ok(msg) => msg,
            Err(broadcast::error::RecvError::Closed) => {
              break;
            },
            Err(broadcast::error::RecvError::Lagged(_)) => {
              warn!("app_launcher receiver lagged");
              continue;
            },
          };

          sender.input(msg);
        }
      })
    };

    let mut list_view_wrapper =
      TypedListView::<AppItem, gtk::SingleSelection>::new();

    let list_view = list_view_wrapper.view.clone();
    let list_view_selection = list_view_wrapper.selection_model.clone();

    list_view.connect_activate({
      let sender = sender.clone();
      move |_, _| {
        sender.input(AppLauncherMsg::Run.into());
      }
    });

    sw.container_add(&list_view);

    list_view_wrapper.set_sorter(|first, second| {
      let first_name = first
        .0
        .name
        .as_deref()
        .unwrap_or_default()
        .to_lowercase();
      let second_name = second
        .0
        .name
        .as_deref()
        .unwrap_or_default()
        .to_lowercase();
      first_name.cmp(&second_name)
    });

    update_list_view(&mut list_view_wrapper, apps);
    {
      let sender = sender.clone();
      input.connect_search_changed(move |input| {
        let text = input.text().to_string();
        sender.input(AppLauncherMsg::UpdateFilter(text).into());
      });
    }

    window.add_controller({
      let key_controller = gtk::EventControllerKey::new();
      let sender = sender.clone();

      let list_view_selection = list_view_selection.clone();

      let move_selection = {
        let list_view = list_view.clone();
        move |delta: i32| {
          let n_items = list_view_selection.model().unwrap().n_items();
          let Ok(index_last) = TryInto::<isize>::try_into(n_items - 1) else {
            return;
          };

          let index = list_view_selection.selected() as isize + delta as isize;
          let index = index.clamp(0, index_last) as u32;
          if list_view_selection.n_items() > 0 {
            list_view_selection.set_selected(index);
            list_view.scroll_to(index, gtk::ListScrollFlags::FOCUS, None);
          }
        }
      };

      key_controller.connect_key_pressed(move |_, keyval, _, state| {
        let get_visible_items = || {
          let viewport_height = list_view.height();

          let item_height = if let Some(first_row) = list_view.first_child() {
            first_row.height()
          } else {
            return None;
          };

          if item_height > 0 {
            Some((viewport_height / item_height).max(1) + 1)
          } else {
            None
          }
        };

        match keyval {
          gdk::Key::Escape => {
            sender.input(AppLauncherMsg::SetVisibility(false).into());
          },
          gdk::Key::Up => move_selection(-1),
          gdk::Key::Down => move_selection(1),
          gdk::Key::Page_Up => {
            if let Some(visible_items) = get_visible_items() {
              move_selection(-visible_items);
            }
          },
          gdk::Key::Page_Down => {
            if let Some(visible_items) = get_visible_items() {
              move_selection(visible_items);
            }
          },
          gdk::Key::Home => move_selection(-i32::MAX),
          gdk::Key::End => move_selection(i32::MAX),
          gdk::Key::Tab | gdk::Key::ISO_Left_Tab => {
            if state.contains(gdk::ModifierType::SHIFT_MASK) {
              move_selection(-1);
            } else {
              move_selection(1);
            }
          },
          gdk::Key::Return => {
            sender.input(AppLauncherMsg::Run.into());
          },
          _ => {
            return glib::Propagation::Proceed;
          },
        }

        glib::Propagation::Stop
      });
      key_controller.set_propagation_phase(gtk::PropagationPhase::Capture);
      key_controller
    });

    let model = AppLauncher {
      visible: init.visible,
      dbus_connection: init.dbus_connection,
      new_apps: None,
      list_view_wrapper,
      input,
      handle: Some(handle),
    };

    let widgets = AppLauncherWidgets { window };

    AsyncComponentParts { model, widgets }
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

  async fn update(
    &mut self,
    msg: Self::Input,
    _sender: AsyncComponentSender<Self>,
    _root: &Self::Root,
  ) {
    match msg {
      Msg::Shutdown => {},
      Msg::AppLauncherMsg(AppLauncherMsg::UpdateApps(new_apps)) => {
        self.new_apps = Some(new_apps);
      },
      Msg::AppLauncherMsg(AppLauncherMsg::UpdateFilter(text)) => {
        let text = text.trim().to_lowercase();

        self.list_view_wrapper.set_sorter({
          let text = text.to_lowercase();
          move |first, second| {
            let first_name = first
              .0
              .name
              .as_deref()
              .unwrap_or_default()
              .to_lowercase();
            let second_name = second
              .0
              .name
              .as_deref()
              .unwrap_or_default()
              .to_lowercase();

            let first_starts_with = first_name.starts_with(&text);
            let second_starts_with = second_name.starts_with(&text);

            match (first_starts_with, second_starts_with) {
              (true, false) => return std::cmp::Ordering::Less,
              (false, true) => return std::cmp::Ordering::Greater,
              _ => {},
            }

            let first_contains = first_name.contains(&text);
            let second_contains = second_name.contains(&text);
            match (first_contains, second_contains) {
              (true, false) => return std::cmp::Ordering::Less,
              (false, true) => return std::cmp::Ordering::Greater,
              _ => {},
            }

            first_name.cmp(&second_name)
          }
        });

        self.list_view_wrapper.clear_filters();
        self.list_view_wrapper.add_filter(move |item| {
          if item
            .0
            .name
            .as_deref()
            .unwrap_or_default()
            .to_lowercase()
            .contains(&text)
          {
            return true;
          }

          for k in item.0.keywords.as_deref().unwrap_or_default() {
            if k.to_lowercase().contains(&text) {
              return true;
            }
          }

          false
        });

        self.select(0);
      },
      Msg::AppLauncherMsg(AppLauncherMsg::SetVisibility(visible)) => {
        self.input.set_text("");
        self.set_visibility(visible);
        self.select(0);
      },
      Msg::AppLauncherMsg(AppLauncherMsg::ToggleVisibility) => {
        self.input.set_text("");
        self.set_visibility(!self.visible);
        self.select(0);
      },
      Msg::AppLauncherMsg(AppLauncherMsg::Run) => {
        let selected = self.list_view_wrapper.selection_model.selected();
        self.set_visibility(false);
        self.select(0);

        if let Some(item) = self.list_view_wrapper.get_visible(selected) {
          let item = item.borrow();
          if let Err(err) = self.launch_app(&item).await {
            warn!("failed to launch {}: {}", item.0.id, err);
          }
        }
      },
    }
  }

  fn update_view(
    &self,
    widgets: &mut Self::Widgets,
    _sender: AsyncComponentSender<Self>,
  ) {
    widgets.window.set_visible(self.visible);
  }
}

pub async fn run(messenger: Messenger) {
  watch_apps(messenger).await
}

async fn watch_apps(mut messenger: Messenger) {
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

  for d in xdg::BASEDIRS.find_data_files("icons") {
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
      _ = tokio::time::sleep(Duration::from_secs(1)) => {
        if !dirty.load(std::sync::atomic::Ordering::SeqCst) {
          continue;
        }
        tracing::info!("timeout after dirty");
      },
    }

    let apps = xdg::application::read_applications()
      .await
      .expect("failed to read applications");
    messenger
      .tx()
      .send(AppLauncherMsg::UpdateApps(apps).into())
      .unwrap();
    dirty.store(false, std::sync::atomic::Ordering::SeqCst);
  }
}
