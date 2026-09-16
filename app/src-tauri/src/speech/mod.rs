//! Speech input for the Jarvis view (centralization Phase 4). Three commands
//! and one event, identical on every platform; only macOS has a real
//! implementation (`macos.rs`), as `CLAUDE.md` prescribes — Windows is
//! text-only and gets the stub below, which reports `stt: false` so the
//! view hides the hold-to-talk affordance.
//!
//! Speech *output* is not here: the webview's `speechSynthesis` does it,
//! cross-platform, with no native code.

use serde::Serialize;

/// Live caption while listening: `{ text }` with the best transcript so far.
pub const PARTIAL_EVENT: &str = "speech-partial";

#[derive(Debug, Clone, Serialize)]
pub struct SpeechSupport {
    pub stt: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
pub use macos::SpeechHandle;

#[cfg(not(target_os = "macos"))]
mod stub;

#[cfg(not(target_os = "macos"))]
pub use stub::SpeechHandle;

/// Whether hold-to-talk will work: recognizer present and authorized (asks
/// the first time). Cheap to call on every mount.
#[tauri::command]
pub async fn speech_support(
    state: tauri::State<'_, crate::AppState>,
) -> Result<SpeechSupport, String> {
    Ok(state.speech.support().await)
}

/// Start capturing the microphone into a recognition request. Emits
/// `speech-partial` as words arrive. Errors if already listening or the
/// platform can't.
#[tauri::command]
pub async fn speech_start(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
) -> Result<(), String> {
    state.speech.start(app).await
}

/// Stop capturing and return the final transcript (`""` if nothing was
/// recognized). Waits briefly for the recognizer's final result.
#[tauri::command]
pub async fn speech_stop(state: tauri::State<'_, crate::AppState>) -> Result<String, String> {
    state.speech.stop().await
}
