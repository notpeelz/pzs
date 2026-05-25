use std::future::Future;
use std::str::FromStr;

use eyre::{Context, Result};
use tokio::sync::broadcast;
use tracing::warn;

use crate::app_launcher::AppLauncherMsg;

#[derive(Debug, Clone)]
pub enum Msg {
  Shutdown,
  AppLauncherMsg(AppLauncherMsg),
}

#[allow(dead_code)]
pub struct Messenger {
  rx: Option<broadcast::Receiver<Msg>>,
  tx: broadcast::Sender<Msg>,
}

impl Clone for Messenger {
  fn clone(&self) -> Self {
    Self {
      tx: self.tx.clone(),
      rx: None,
    }
  }
}

impl Messenger {
  pub fn tx(&mut self) -> &mut broadcast::Sender<Msg> {
    &mut self.tx
  }

  pub fn rx(&mut self) -> &mut broadcast::Receiver<Msg> {
    if self.rx.is_none() {
      self.rx = Some(self.tx.subscribe());
    }

    self.rx.as_mut().unwrap()
  }

  pub fn send_msg<M: Into<Msg>>(&self, msg: M) -> zbus::fdo::Result<()> {
    self.tx.send(msg.into()).map(|_| ()).map_err(|_| {
      zbus::fdo::Error::Failed("failed to dispatch message".into())
    })
  }

  pub fn new(capacity: usize) -> Self {
    let (tx, rx) = broadcast::channel::<Msg>(capacity);
    Self { tx, rx: Some(rx) }
  }
}

pub async fn setup(
  mut messenger: Messenger,
) -> Result<(zbus::Connection, impl Future<Output = ()>)> {
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
    crate::ShellInterface {
      messenger: messenger.clone(),
    }
  })
  .wrap_err("failed to register ShellInterface on bus")?
  .serve_at(PATH, {
    crate::app_launcher::AppLauncherInterface {
      messenger: messenger.clone(),
    }
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
      match messenger.rx().recv().await {
        Ok(Msg::Shutdown) | Err(broadcast::error::RecvError::Closed) => break,
        Err(broadcast::error::RecvError::Lagged(_)) => {
          warn!("dbus receiver lagged");
        },
        _ => (),
      };
    }
  };

  Ok((connection, connection_guard))
}
