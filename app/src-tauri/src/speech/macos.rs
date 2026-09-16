//! macOS speech input: `SFSpeechRecognizer` fed by an `AVAudioEngine`
//! microphone tap, through the `objc2-speech` / `objc2-avf-audio` bindings.
//!
//! Every Objective-C object lives on one dedicated OS thread (the "speech
//! thread"), driven by a command channel. `Retained<_>` is `!Send` and Tauri
//! commands run on runtime threads, so a single owning thread is the whole
//! concurrency story: commands send a `Cmd` with a reply channel and block
//! (off the async runtime) until it answers. Only `Send` things — an
//! `AppHandle`, `Arc<Latest>` — are captured by the recognition blocks, which
//! AVFoundation and Speech invoke on their own queues.
//!
//! Requires `NSMicrophoneUsageDescription` and
//! `NSSpeechRecognitionUsageDescription` in `Info.plist` (they are), or the
//! process is terminated on first use.

use super::{SpeechSupport, PARTIAL_EVENT};
use block2::RcBlock;
use objc2::rc::Retained;
use objc2_avf_audio::{AVAudioEngine, AVAudioPCMBuffer, AVAudioTime};
use objc2_foundation::NSError;
use objc2_speech::{
    SFSpeechAudioBufferRecognitionRequest, SFSpeechRecognitionResult, SFSpeechRecognitionTask,
    SFSpeechRecognizer, SFSpeechRecognizerAuthorizationStatus,
};
use std::ptr::NonNull;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

/// How long `stop` waits for the recognizer's final result after `endAudio`.
const FINAL_RESULT_WAIT: Duration = Duration::from_secs(3);
/// The authorization prompt is modal to the user; give them time.
const AUTHORIZATION_WAIT: Duration = Duration::from_secs(120);

enum Cmd {
    Support(Sender<SpeechSupport>),
    Start(AppHandle, Sender<Result<(), String>>),
    Stop(Sender<Result<String, String>>),
}

/// Cross-thread front for the speech thread. Lives in `AppState`.
pub struct SpeechHandle {
    tx: Mutex<Option<Sender<Cmd>>>,
}

impl SpeechHandle {
    pub fn new() -> Self {
        SpeechHandle {
            tx: Mutex::new(None),
        }
    }

    /// The speech thread is spawned on first use, not at app start: most
    /// launches never open the Jarvis tab, and the first `SFSpeechRecognizer`
    /// touch is what triggers the authorization prompt.
    fn sender(&self) -> Sender<Cmd> {
        let mut guard = self.tx.lock().expect("speech mutex poisoned");
        guard
            .get_or_insert_with(|| {
                let (tx, rx) = mpsc::channel();
                std::thread::Builder::new()
                    .name("speech".into())
                    .spawn(move || Worker::run(rx))
                    .expect("spawn speech thread");
                tx
            })
            .clone()
    }

    async fn ask<T: Send + 'static>(
        &self,
        make: impl FnOnce(Sender<T>) -> Cmd,
        timeout: Duration,
        on_silence: impl FnOnce() -> T + Send + 'static,
    ) -> T {
        let (reply_tx, reply_rx) = mpsc::channel();
        let _ = self.sender().send(make(reply_tx));
        tauri::async_runtime::spawn_blocking(move || {
            reply_rx
                .recv_timeout(timeout)
                .unwrap_or_else(|_| on_silence())
        })
        .await
        .unwrap_or_else(|_| unreachable!("speech reply task panicked"))
    }

    pub async fn support(&self) -> SpeechSupport {
        self.ask(
            Cmd::Support,
            AUTHORIZATION_WAIT + Duration::from_secs(5),
            || SpeechSupport {
                stt: false,
                reason: Some("speech thread did not answer".to_owned()),
            },
        )
        .await
    }

    pub async fn start(&self, app: AppHandle) -> Result<(), String> {
        self.ask(
            |tx| Cmd::Start(app, tx),
            AUTHORIZATION_WAIT + Duration::from_secs(5),
            || Err("speech thread did not answer".to_owned()),
        )
        .await
    }

    pub async fn stop(&self) -> Result<String, String> {
        self.ask(
            Cmd::Stop,
            FINAL_RESULT_WAIT + Duration::from_secs(2),
            || Err("speech thread did not answer".to_owned()),
        )
        .await
    }
}

/// The recognizer's latest word on what was said, shared with its result
/// handler. `done` flips on the final result or an error, waking `stop`.
#[derive(Default)]
struct Latest {
    state: Mutex<LatestState>,
    cv: Condvar,
}

#[derive(Default)]
struct LatestState {
    text: String,
    done: bool,
}

impl Latest {
    fn update(&self, text: String, done: bool) {
        let mut s = self.state.lock().expect("latest mutex poisoned");
        if !text.is_empty() {
            s.text = text;
        }
        s.done |= done;
        self.cv.notify_all();
    }

    fn finish(&self) {
        let mut s = self.state.lock().expect("latest mutex poisoned");
        s.done = true;
        self.cv.notify_all();
    }

    fn wait(&self, timeout: Duration) -> String {
        let guard = self.state.lock().expect("latest mutex poisoned");
        let (s, _) = self
            .cv
            .wait_timeout_while(guard, timeout, |s| !s.done)
            .expect("latest mutex poisoned");
        s.text.clone()
    }
}

/// One hold-to-talk capture.
struct Capture {
    request: Retained<SFSpeechAudioBufferRecognitionRequest>,
    task: Retained<SFSpeechRecognitionTask>,
    latest: Arc<Latest>,
}

struct Worker {
    engine: Retained<AVAudioEngine>,
    recognizer: Option<Retained<SFSpeechRecognizer>>,
    capture: Option<Capture>,
}

impl Worker {
    fn run(rx: Receiver<Cmd>) {
        let mut w = Worker {
            engine: unsafe { AVAudioEngine::new() },
            recognizer: None,
            capture: None,
        };
        for cmd in rx {
            match cmd {
                Cmd::Support(reply) => {
                    let _ = reply.send(w.support());
                }
                Cmd::Start(app, reply) => {
                    let _ = reply.send(w.start(app));
                }
                Cmd::Stop(reply) => {
                    let _ = reply.send(w.stop());
                }
            }
        }
    }

    fn support(&mut self) -> SpeechSupport {
        use SFSpeechRecognizerAuthorizationStatus as S;
        let unsupported = |reason: &str| SpeechSupport {
            stt: false,
            reason: Some(reason.to_owned()),
        };
        let mut status = unsafe { SFSpeechRecognizer::authorizationStatus() };
        if status == S::NotDetermined {
            let (tx, rx) = mpsc::channel();
            let block: RcBlock<dyn Fn(S)> = RcBlock::new(move |s: S| {
                let _ = tx.send(s);
            });
            unsafe { SFSpeechRecognizer::requestAuthorization(&block) };
            status = rx
                .recv_timeout(AUTHORIZATION_WAIT)
                .unwrap_or(S::NotDetermined);
        }
        match status {
            S::Authorized => {}
            S::Denied => {
                return unsupported(
                    "speech recognition is denied for J.A.R.V.I.S. in System Settings → Privacy & Security",
                )
            }
            S::Restricted => return unsupported("speech recognition is restricted on this Mac"),
            _ => return unsupported("speech recognition permission not decided yet"),
        }
        let recognizer = self
            .recognizer
            .get_or_insert_with(|| unsafe { SFSpeechRecognizer::new() });
        if !unsafe { recognizer.isAvailable() } {
            return unsupported("the speech recognizer is unavailable right now (offline?)");
        }
        SpeechSupport {
            stt: true,
            reason: None,
        }
    }

    fn start(&mut self, app: AppHandle) -> Result<(), String> {
        if self.capture.is_some() {
            return Err("already listening".to_owned());
        }
        let support = self.support();
        if !support.stt {
            return Err(support
                .reason
                .unwrap_or_else(|| "speech unavailable".to_owned()));
        }
        let recognizer = self.recognizer.as_ref().expect("set by support()");

        unsafe {
            let request = SFSpeechAudioBufferRecognitionRequest::new();
            request.setShouldReportPartialResults(true);

            // Microphone → recognition request, buffer by buffer. AVFoundation
            // copies the block, so our reference can go out of scope.
            let input = self.engine.inputNode();
            let format = input.outputFormatForBus(0);
            let sink = request.clone();
            let tap: RcBlock<dyn Fn(NonNull<AVAudioPCMBuffer>, NonNull<AVAudioTime>)> =
                RcBlock::new(
                    move |buffer: NonNull<AVAudioPCMBuffer>, _when: NonNull<AVAudioTime>| {
                        sink.appendAudioPCMBuffer(buffer.as_ref());
                    },
                );
            input.installTapOnBus_bufferSize_format_block(
                0,
                1024,
                Some(&format),
                RcBlock::as_ptr(&tap),
            );

            self.engine.prepare();
            if let Err(e) = self.engine.startAndReturnError() {
                input.removeTapOnBus(0);
                return Err(format!("microphone: {}", e.localizedDescription()));
            }

            let latest = Arc::new(Latest::default());
            let latest_for_handler = latest.clone();
            let handler: RcBlock<dyn Fn(*mut SFSpeechRecognitionResult, *mut NSError)> =
                RcBlock::new(
                    move |result: *mut SFSpeechRecognitionResult, error: *mut NSError| {
                        if let Some(result) = result.as_ref() {
                            let text = result.bestTranscription().formattedString().to_string();
                            let _ = app.emit(PARTIAL_EVENT, serde_json::json!({ "text": text }));
                            latest_for_handler.update(text, result.isFinal());
                        }
                        if !error.is_null() {
                            // Includes the "canceled" error our own `stop` provokes.
                            latest_for_handler.finish();
                        }
                    },
                );
            let task = recognizer.recognitionTaskWithRequest_resultHandler(&request, &handler);

            self.capture = Some(Capture {
                request,
                task,
                latest,
            });
        }
        Ok(())
    }

    fn stop(&mut self) -> Result<String, String> {
        let Some(capture) = self.capture.take() else {
            return Err("not listening".to_owned());
        };
        unsafe {
            self.engine.stop();
            self.engine.inputNode().removeTapOnBus(0);
            capture.request.endAudio();
        }
        let text = capture.latest.wait(FINAL_RESULT_WAIT);
        // No-op if the task already finished; otherwise stops a straggler
        // from holding the recognizer for the next capture.
        unsafe { capture.task.cancel() };
        Ok(text)
    }
}
