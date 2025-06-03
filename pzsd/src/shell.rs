// SEE ~/code/public/glib/glib/gshell.c
fn tokenize_command<S: AsRef<str>>(s: S) -> Vec<String> {
  let s = s.as_ref();

  let mut tokens = Vec::<String>::new();
  let mut current_token = None::<String>;
  let mut current_escape = None;
  let mut escaped = false;
  let mut chars = s.chars();
  let mut previous_char = None;

  macro_rules! append {
    ($s:tt) => {
      if let Some(ref mut t) = current_token.as_mut() {
        t.push($s);
      } else {
        current_token = Some(::std::string::ToString::to_string(&$s));
      }
    };
  }

  macro_rules! push {
    () => {
      if let Some(token) = current_token.take() {
        tokens.push(token);
      }
    };
  }

  while let Some(c) = chars.next() {
    match current_escape {
      Some('\\') => {
        if c == '\n' {
          // backslash-newline becomes nothing
        } else {
          append!('\\');
          append!(c);
        }

        current_escape = None;
      },
      Some('#') => {
        // Discard up to and including the next newline
        loop {
          let Some(c) = chars.next() else {
            break;
          };

          if c == '\n' {
            break;
          }
        }

        current_escape = None;
      },
      Some(q) => {
        if c == q && !(matches!(current_escape, Some('"')) && escaped) {
          current_escape = None;
        }

        append!(c);
      },
      None => match c {
        '\n' | '\0' => {
          push!();
        },
        ' ' | '\t' => {
          if !current_token
            .as_deref()
            .map(|x| x.is_empty())
            .unwrap_or_default()
          {
            push!();
          }
        },
        '\'' | '"' => {
          append!(c);
          current_escape = Some(c);
        },
        '\\' => {
          current_escape = Some(c);
        },
        '#' => match previous_char {
          None => {
            current_escape = Some(c);
          },
          Some(' ' | '\n' | '\0') => {
            current_escape = Some(c);
          },
          Some(_) => {
            append!(c);
          },
        },
        _ => {
          append!(c);
        },
      },
    }

    // Count consecutive backslashes (mod 2) to detect escaped double quotes.
    if c != '\\' {
      escaped = false;
    } else {
      escaped = !escaped;
    }

    previous_char = Some(c);
  }

  push!();

  match current_escape {
    Some('\\') => {
      // TODO: error 'Text ended after an escape character'
      todo!("Text ended after an escape character")
    },
    Some(_) => {
      // TODO: error 'Text ended before matching quote was found for %c.'
      todo!("Text ended before matching quote was found")
    },
    None => {},
  }

  if tokens.is_empty() {
    todo!("Text was empty (or contained only whitespace)")
  }

  tokens
}

pub fn shell_parse_args<S: AsRef<str>>(s: S) -> Vec<String> {
  let s = s.as_ref();
  tokenize_command(s)
    .iter()
    .map(shell_unquote)
    .collect()
}

pub fn shell_quote<S: AsRef<str>>(s: S) -> String {
  let s = s.as_ref();
  let mut out = String::new();
  out.push('\'');

  let mut chars = s.chars();
  while let Some(c) = chars.next() {
    if c == '\'' {
      out.push_str("'\\''");
    } else {
      out.push(c);
    }
  }

  out.push('\'');

  out
}

pub fn shell_unquote<S: AsRef<str>>(s: S) -> String {
  let s = s.as_ref();
  // TODO: don't use gio
  gio::glib::shell_unquote(s)
    .unwrap()
    .to_string_lossy()
    .to_string()
}
