use core::str;
use std::{
  collections::{HashMap, VecDeque},
  path::PathBuf,
  sync::LazyLock,
};

use eyre::{bail, eyre, Context, OptionExt, Result};
use ini::Ini;
use itertools::Itertools;
use regex::Regex;
use tokio::io::AsyncReadExt;
use tracing::warn;
use walkdir::WalkDir;

use crate::shell::shell_quote;

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
    let exec = entry
      .get("Exec")
      .map(|x| parse_property_string(x))
      .transpose()?;
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

  pub fn expand_exec(&self, params: &[ApplicationParameter]) -> Result<String> {
    if let Some(exec) = &self.exec {
      expand_application_parameters(exec, params)
    } else {
      bail!("app doesn't have an Exec property");
    }
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
        let file = match tokio::fs::File::open(&desktop_path).await {
          Ok(file) => file,
          Err(err) => {
            warn!("failed to read desktop file {desktop_path:?}: {err:?}");
            continue;
          },
        };
        Application::from_reader(&app_id, file).await
      } {
        Ok(f) => f,
        Err(err) => {
          let err = err.wrap_err(format!(
            "failed to read desktop file: {}",
            desktop_path.to_string_lossy()
          ));
          warn!("{err}");
          continue;
        },
      };

      m.insert(app_id.clone(), desktop_file);
    }
  }

  Ok(m.into_values().collect::<Vec<_>>())
}

pub fn parse_property_string(s: &str) -> Result<String> {
  static RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\\[sntr\\0]?"#).unwrap());

  let mut result = String::with_capacity(s.len());
  let mut last_match = 0;
  for caps in RE.captures_iter(s) {
    let m = caps.get(0).unwrap();
    result.push_str(&s[last_match..m.start()]);
    result.push_str({
      match m.as_str() {
        "\\s" => " ",
        "\\n" => "\n",
        "\\t" => "\t",
        "\\r" => "\r",
        "\\\\" => "\\",
        "\\" => bail!("property contains escape character at end of line"),
        _ => panic!(),
      }
    });
    last_match = m.end();
  }
  result.push_str(&s[last_match..]);
  Ok(result.to_string())
}

pub enum ApplicationParameter {
  Uri(String),
  File(PathBuf),
}

impl ApplicationParameter {
  fn to_uri(&self) -> String {
    match self {
      ApplicationParameter::Uri(uri) => uri.to_owned(),
      ApplicationParameter::File(path_buf) => {
        // TODO: is this correct? should we escape certain chars?
        format!("file://{}", path_buf.to_string_lossy())
      },
    }
  }
}

pub fn expand_application_parameters<S: AsRef<str>>(
  s: S,
  params: &[ApplicationParameter],
) -> Result<String> {
  let s = s.as_ref();
  let params_count = params.len();
  let mut params = VecDeque::from_iter(params.iter());

  let mut out = String::new();
  let mut chars = s.chars().peekable();
  while let Some(c) = chars.peek() {
    let c = *c;
    if c == '%' {
      chars.next();
      let Some(field) = chars.next() else {
        out.push(c);
        continue;
      };
      match field {
        'u' => {
          if let Some(p) = params.pop_front() {
            out.push_str(&shell_quote(&p.to_uri()));
          }
        },
        'U' => {
          if !params.is_empty() {
            out.push_str(
              &params
                .drain(..)
                .map(|p| shell_quote(&p.to_uri()))
                .join(" "),
            )
          }
        },
        'f' => {
          if let Some(p) = params.pop_front() {
            out.push_str(&shell_quote(match p {
              ApplicationParameter::Uri(_) => {
                bail!("parameter is not a path");
              },
              ApplicationParameter::File(path) => path.to_string_lossy(),
            }));
          }
        },
        'F' => {
          if !params.is_empty() {
            out.push_str(
              &params
                .drain(..)
                .map(|p| {
                  Ok(shell_quote(match p {
                    ApplicationParameter::Uri(_) => {
                      bail!("parameter is not a path");
                    },
                    ApplicationParameter::File(path) => path.to_string_lossy(),
                  }))
                })
                .collect::<Result<Vec<_>>>()?
                .join(" "),
            )
          }
        },
        _ => {},
      }
    } else {
      out.push(c);
    }
    chars.next();
  }

  // Default to appending '%f' if there were no substitutions.
  if !params.is_empty() && params_count == params.len() {
    out.push(' ');
    if let Some(p) = params.pop_front() {
      out.push_str(&shell_quote(match p {
        ApplicationParameter::Uri(_) => {
          bail!("parameter is not a path");
        },
        ApplicationParameter::File(path) => path.to_string_lossy(),
      }));
    }
  }

  Ok(out)
}
