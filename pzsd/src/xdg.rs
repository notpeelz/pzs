mod application;

pub use application::read_applications;
pub use application::Application;
use once_cell::sync::Lazy;

pub static BASEDIRS: Lazy<::xdg::BaseDirectories> =
  Lazy::new(|| ::xdg::BaseDirectories::new().unwrap());
