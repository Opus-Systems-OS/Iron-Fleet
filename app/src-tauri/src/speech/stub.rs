//! Non-macOS: no speech input. The view stays usable through its text box.

use super::SpeechSupport;

pub struct SpeechHandle;

impl SpeechHandle {
    pub fn new() -> Self {
        SpeechHandle
    }

    pub async fn support(&self) -> SpeechSupport {
        SpeechSupport {
            stt: false,
            reason: Some("speech input is macOS-only for now; type instead".to_owned()),
        }
    }

    pub async fn start(&self, _app: tauri::AppHandle) -> Result<(), String> {
        Err("speech input is not available on this platform".to_owned())
    }

    pub async fn stop(&self) -> Result<String, String> {
        Err("speech input is not available on this platform".to_owned())
    }
}
