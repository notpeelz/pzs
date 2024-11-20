use std::{collections::HashMap, path::PathBuf};

use eyre::Result;
use itertools::Itertools;
use once_cell::sync::Lazy;
use walkdir::WalkDir;

pub static BASEDIRS: Lazy<::xdg::BaseDirectories> =
  Lazy::new(|| ::xdg::BaseDirectories::new().unwrap());

#[derive(Debug)]
pub struct Application {
  pub id: String,
  pub desktop_file: DesktopFile,
}

#[derive(Debug)]
pub struct DesktopFile {
  pub path: PathBuf,
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
        let path = path
          .strip_prefix(&d)
          // this shouldn't ever panic
          .unwrap();

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

      m.insert(
        app_id.clone(),
        Application {
          id: app_id,
          desktop_file: DesktopFile {
            path: entry.into_path(),
          },
        },
      );
    }
  }

  Ok(m.into_values().collect::<Vec<_>>())
}
