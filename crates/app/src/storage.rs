//! Persistence: the auto-saved session, and saving/opening `.json` patch
//! files.
//!
//! * Web: the session lives in `localStorage`; files are downloaded through
//!   a temporary link and opened with a file `<input>`.
//! * Desktop: the session lives in `~/.config/kickwerk/session.json`; files
//!   go through the native file dialogs.

use crate::patch::PatchData;

#[cfg(all(feature = "web", not(feature = "desktop")))]
mod imp {
    use wasm_bindgen::JsCast;

    const SESSION_KEY: &str = "kickwerk.session";

    pub fn load_session() -> Option<String> {
        let storage = web_sys::window()?.local_storage().ok()??;
        storage.get_item(SESSION_KEY).ok()?
    }

    pub fn save_session(json: &str) {
        if let Some(Ok(Some(storage))) = web_sys::window().map(|w| w.local_storage()) {
            let _ = storage.set_item(SESSION_KEY, json);
        }
    }

    /// Offer `json` as a download named `file_name`.
    pub async fn save_file(file_name: &str, json: &str) -> Result<Option<String>, String> {
        let err = |e: wasm_bindgen::JsValue| format!("{e:?}");
        let parts = js_sys::Array::of1(&wasm_bindgen::JsValue::from_str(json));
        let props = web_sys::BlobPropertyBag::new();
        props.set_type("application/json");
        let blob = web_sys::Blob::new_with_str_sequence_and_options(&parts, &props).map_err(err)?;
        let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(err)?;
        let document = web_sys::window().and_then(|w| w.document()).ok_or("no document")?;
        let a: web_sys::HtmlAnchorElement = document
            .create_element("a")
            .map_err(err)?
            .dyn_into()
            .map_err(|_| "no anchor".to_string())?;
        a.set_href(&url);
        a.set_download(file_name);
        if let Some(body) = document.body() {
            let _ = body.append_child(&a);
            a.click();
            let _ = body.remove_child(&a);
        }
        let _ = web_sys::Url::revoke_object_url(&url);
        Ok(Some(file_name.to_string()))
    }
}

#[cfg(not(all(feature = "web", not(feature = "desktop"))))]
mod imp {
    use std::path::PathBuf;

    fn config_dir() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("kickwerk"))
    }

    pub fn load_session() -> Option<String> {
        std::fs::read_to_string(config_dir()?.join("session.json")).ok()
    }

    pub fn save_session(json: &str) {
        let Some(dir) = config_dir() else { return };
        let result = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(dir.join("session.json"), json));
        if let Err(e) = result {
            eprintln!("kickwerk: could not save the session in {}: {e}", dir.display());
        }
    }

    /// `rfd` answers "cancelled" immediately when there is no file dialog
    /// at all (no XDG portal and no zenity); a person takes longer than this.
    #[cfg(feature = "desktop")]
    const NO_DIALOG: std::time::Duration = std::time::Duration::from_millis(250);

    /// Where patches go when there is no file dialog to ask.
    #[cfg(feature = "desktop")]
    pub fn fallback_dir() -> Option<PathBuf> {
        let home = PathBuf::from(std::env::var_os("HOME")?);
        let docs = home.join("Documents");
        Some(if docs.is_dir() { docs } else { home }.join("Kickwerk"))
    }

    /// Ask where to save, then write. `Ok(None)` if the dialog was
    /// cancelled. Without a file dialog the patch goes to [`fallback_dir`].
    #[cfg(feature = "desktop")]
    pub async fn save_file(file_name: &str, json: &str) -> Result<Option<String>, String> {
        let asked = std::time::Instant::now();
        let picked = rfd::AsyncFileDialog::new()
            .set_title("Save Kickwerk patch")
            .set_file_name(file_name)
            .add_filter("Kickwerk patch", &["json"])
            .save_file()
            .await;
        let path = match picked {
            Some(handle) => handle.path().to_path_buf(),
            None if asked.elapsed() < NO_DIALOG => {
                let dir = fallback_dir().ok_or("no home directory")?;
                std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
                dir.join(file_name)
            }
            None => return Ok(None),
        };
        std::fs::write(&path, json).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Some(path.display().to_string()))
    }

    /// Ask for a patch file and read it. `None` if the dialog was cancelled.
    #[cfg(feature = "desktop")]
    pub async fn open_file() -> Option<Result<(String, String), String>> {
        let asked = std::time::Instant::now();
        let Some(handle) = rfd::AsyncFileDialog::new()
            .set_title("Open Kickwerk patch")
            .add_filter("Kickwerk patch", &["json"])
            .pick_file()
            .await
        else {
            return (asked.elapsed() < NO_DIALOG)
                .then(|| Err("no file dialog available (install xdg-desktop-portal or zenity)".to_string()));
        };
        let path = handle.path().to_path_buf();
        Some(
            std::fs::read_to_string(&path)
                .map(|text| (path.display().to_string(), text))
                .map_err(|e| format!("{}: {e}", path.display())),
        )
    }

    #[cfg(not(feature = "desktop"))]
    pub async fn save_file(_file_name: &str, _json: &str) -> Result<Option<String>, String> {
        Err("no file support in this build".into())
    }
}

#[cfg(feature = "desktop")]
pub use imp::open_file;
pub use imp::save_file;

pub fn load_session() -> Option<PatchData> {
    let text = imp::load_session()?;
    match PatchData::from_json(&text) {
        Ok(p) => Some(p),
        Err(e) => {
            eprintln!("kickwerk: ignoring saved session: {e}");
            None
        }
    }
}

pub fn save_session(json: &str) {
    imp::save_session(json);
}

/// A file name for a patch: its name, lower-cased and hyphenated.
pub fn file_name(name: &str) -> String {
    let slug: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        "kickwerk-patch.json".into()
    } else {
        format!("{slug}.json")
    }
}

#[cfg(test)]
mod tests {
    use super::file_name;

    #[test]
    fn file_names() {
        assert_eq!(file_name("Berlin Rumble"), "berlin-rumble.json");
        assert_eq!(file_name("  "), "kickwerk-patch.json");
        assert_eq!(file_name("Kick #3 / final!"), "kick-3-final.json");
    }
}
