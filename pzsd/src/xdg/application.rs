use core::str;
use std::{collections::HashMap, path::PathBuf};

use eyre::{Context, OptionExt, Result};
use ini::Ini;
use itertools::Itertools;
use tokio::io::AsyncReadExt;
use tracing::warn;
use walkdir::WalkDir;

use super::BASEDIRS;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Application {
  pub id: String,
  pub no_display: Option<bool>,
  pub icon: Option<String>,
  pub name: Option<String>,
  pub exec: Option<String>,
  pub keywords: Option<Vec<String>>,
  pub only_show_in: Option<Vec<String>>,
}

impl Application {
  async fn from_reader<S: ToString, R: tokio::io::AsyncRead + Unpin>(
    id: S,
    mut r: R,
  ) -> Result<Application> {
    let id = id.to_string();

    let mut buf = Vec::<u8>::new();
    r.read_to_end(&mut buf).await?;

    let ini = {
      let s = str::from_utf8(&buf)
        .wrap_err("file contains invalid utf-8 characters")?;
      Ini::load_from_str_opt(
        s,
        ini::ParseOption {
          enabled_escape: true,
          enabled_quote: false,
        },
      )
      .wrap_err("failed to parse file")?
    };

    let entry = ini
      .section(Some("Desktop Entry"))
      .ok_or_eyre("section 'Desktop Entry' doesn't exist")?;

    let no_display = match entry
      .get("NoDisplay")
      .map(|x| x.to_ascii_lowercase())
      .as_deref()
    {
      Some("true") => Some(true),
      Some("false") => Some(false),
      _ => None,
    };

    let icon = entry.get("Icon").map(|x| x.to_owned());
    let name = entry.get("Name").map(|x| x.to_owned());
    let exec = entry.get("Exec").map(|x| x.to_owned());
    let keywords = entry.get("Keywords").map(|x| {
      x.split(";")
        .map(|k| k.to_owned())
        .collect::<Vec<_>>()
    });
    let only_show_in = entry.get("OnlyShowIn").map(|x| {
      x.split(";")
        .map(|k| k.to_owned())
        .collect::<Vec<_>>()
    });

    Ok(Application {
      id,
      no_display,
      icon,
      name,
      exec,
      keywords,
      only_show_in,
    })
  }
}

pub async fn read_applications() -> Result<Vec<Application>> {
  let mut m = HashMap::<String, Application>::new();
  for d in BASEDIRS.find_data_files("applications") {
    WalkDir::new(&d).follow_links(true).into_iter();
    let mut dir = WalkDir::new(&d)
      .follow_links(true)
      .into_iter()
      .filter_map(|e| e.ok());
    while let Some(entry) = dir.next() {
      let Some(filename) = entry.file_name().to_str() else {
        // ignore non-utf8 file names
        continue;
      };

      let Some((stem, ext)) = filename.rsplit_once('.') else {
        continue;
      };

      if ext != "desktop" {
        continue;
      }

      let app_id = {
        let path = entry.path();
        let path = path.strip_prefix(&d).unwrap();

        // remove file name
        let count = path.components().count();
        let path = path
          .components()
          .take(count - 1)
          .collect::<PathBuf>();

        path
          .components()
          .filter_map(|x| x.as_os_str().to_str())
          .chain([stem])
          .intersperse("-")
          .collect::<String>()
      };

      let desktop_path = entry.into_path();

      let desktop_file = match {
        let r = tokio::fs::File::open(&desktop_path).await?;

        Application::from_reader(&app_id, r)
          .await
          .wrap_err("invalid desktop file")
      } {
        Ok(f) => f,
        Err(r) => {
          let r = r.wrap_err(format!(
            "failed to read desktop file: {}",
            desktop_path.to_string_lossy()
          ));
          warn!("{r}");
          continue;
        },
      };

      m.insert(app_id.clone(), desktop_file);
    }
  }

  Ok(m.into_values().collect::<Vec<_>>())
}

fn expand_application_parameters<S: AsRef<str>>(s: S) -> String {
  // 1. g_key_file_parse_value_as_string - replace \n, \s, \t, etc
  // 2. expand_application_parameters - substitute field codes (%f, etc)
  // 3. g_shell_parse_argv - split into arg array
  let s = s.as_ref();

  todo!()
}
