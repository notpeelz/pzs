use std::{sync::atomic::AtomicBool, time::Duration};

use crate::{
  list::{RelmListItem, TypedListView},
  main::Msg,
  xdg, XDG_CURRENT_DESKTOP,
};
use eyre::bail;
use eyre::Result;
use gio::{glib, prelude::ListModelExt};
use gtk4::{
  self as gtk,
  prelude::{EditableExt, EventControllerExt},
  IconPaintable,
};
use gtk4::{gdk, prelude::WidgetExt};
use gtk4_layer_shell::{Layer, LayerShell};
use notify::Watcher;
use relm4::{
  prelude::{AsyncComponent, AsyncComponentParts},
  AsyncComponentSender, RelmContainerExt, RelmWidgetExt,
};
use tokio::sync::broadcast;
use tracing::{info, warn};

pub struct AppLauncherInit {
  pub visible: bool,
  pub msg_tx: broadcast::Sender<Msg>,
  pub msg_rx: broadcast::Receiver<Msg>,
}

pub struct AppLauncher {
  visible: bool,
  new_apps: Option<Vec<xdg::Application>>,
  list_view_wrapper: TypedListView<AppItem, gtk::SingleSelection>,
  input: gtk::SearchEntry,
  handle: Option<glib::JoinHandle<()>>,
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
struct AppItem {
  id: String,
  icon: String,
  name: String,
  keywords: Vec<String>,
}

struct AppItemWidgets {
  image: gtk::Image,
  label: gtk::Label,
}

impl Drop for AppItemWidgets {
  fn drop(&mut self) {
    // dbg!(self.label.label());
  }
}

impl RelmListItem for AppItem {
  type Root = gtk::Box;
  type Widgets = AppItemWidgets;

  fn setup(_item: &gtk::ListItem) -> (gtk::Box, AppItemWidgets) {
    relm4::view! {
      my_box = gtk::Box {
        #[name = "image"]
        gtk::Image,

        #[name = "label"]
        gtk::Label,
      }
    }

    image.set_pixel_size(48);

    let widgets = AppItemWidgets { image, label };

    (my_box, widgets)
  }

  fn bind(&mut self, widgets: &mut Self::Widgets, _root: &mut Self::Root) {
    let AppItemWidgets { image, label } = widgets;

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
pub struct AppLauncherWidgets {
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
  apps: &[xdg::Application],
) {
  list_view.clear();
  for app in apps.into_iter() {
    let desktop_file = &app.desktop_file;

    let no_display = desktop_file.no_display.unwrap_or(false);
    if no_display {
      continue;
    }

    if let Some(only_show_in) = &desktop_file.only_show_in {
      if let Some(desktop) = &*XDG_CURRENT_DESKTOP {
        if !only_show_in.contains(desktop) {
          continue;
        }
      }
    }

    let Some(ref icon) = desktop_file.icon else {
      continue;
    };
    let Some(ref name) = desktop_file.name else {
      continue;
    };
    let keywords = desktop_file.keywords.clone().unwrap_or_default();

    list_view.append(AppItem {
      id: app.id.clone(),
      icon: icon.clone(),
      name: name.clone(),
      keywords,
    });
  }
}

fn launch_app<S: AsRef<str>>(id: S) -> Result<()> {
  let id = id.as_ref();
  info!("launching app \"{}\"", id);

  let id = format!("{id}.desktop");
  let id = std::ffi::CString::new(id)?;

  match unsafe { nix::unistd::fork() } {
    Ok(nix::unistd::ForkResult::Child) => {
      let _ = nix::unistd::setsid();

      match unsafe { nix::unistd::fork() } {
        Ok(nix::unistd::ForkResult::Child) => unsafe {
          let fd = libc::open(c"/dev/null".as_ptr(), libc::O_RDWR);
          if fd < 0 {
            libc::_exit(0);
          }

          if libc::dup2(fd, libc::STDOUT_FILENO) < 0
            || libc::dup2(fd, libc::STDERR_FILENO) < 0
            || libc::dup2(fd, libc::STDIN_FILENO) < 0
          {
            libc::_exit(0);
          }

          libc::close(fd);
          let _ = nix::unistd::execvp(c"uwsm", &[c"uwsm", c"app", c"--", &id]);

          libc::_exit(0);
        },
        Ok(nix::unistd::ForkResult::Parent { .. }) => {},
        Err(_) => {},
      }

      unsafe { libc::_exit(0) };
    },
    Ok(nix::unistd::ForkResult::Parent { child }) => {
      nix::sys::wait::waitpid(child, None).unwrap();
    },
    Err(err) => {
      bail!("fork failed: {}", err.desc());
    },
  }

  Ok(())
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
    let apps = xdg::read_applications()
      .await
      .expect("failed to read applications");

    relm4::set_global_css(
      "listview > row:selected {
        background-color: #3b3e9380;
      }",
    );

    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_visible(init.visible);
    window.set_keyboard_mode(gtk4_layer_shell::KeyboardMode::OnDemand);

    window.inline_css("border-radius: 12px 12px 12px 12px");
    window.inline_css("border: 2px solid #3b3e93");

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
      .build();

    input.inline_css("border: 2px solid #3b3e93");
    input.inline_css("border-radius: 50px");
    input.inline_css("outline: none");

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
      let first_name = first.name.to_lowercase();
      let second_name = second.name.to_lowercase();
      first_name.cmp(&second_name)
    });

    update_list_view(&mut list_view_wrapper, &apps);
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
            let first_name = first.name.to_lowercase();
            let second_name = second.name.to_lowercase();

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
          if item.name.to_lowercase().contains(&text) {
            return true;
          }

          for k in &item.keywords {
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
          if let Err(err) = launch_app(&item.id) {
            warn!("failed to launch {}: {}", item.id, err);
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

pub async fn run(msg_tx: broadcast::Sender<Msg>) {
  watch_apps(msg_tx).await
}

async fn watch_apps(msg_tx: broadcast::Sender<Msg>) {
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
    msg_tx
      .send(AppLauncherMsg::UpdateApps(apps).into())
      .unwrap();
    dirty.store(false, std::sync::atomic::Ordering::SeqCst);
  }
}
