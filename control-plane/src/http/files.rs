//! Files: uploads a client attaches to a session message, and the files an
//! agent hands back by writing to `/mnt/session/outputs/` (docs:
//! managed-agents/files).
//!
//! An upload goes to the Files API from here — only the control plane holds
//! the Anthropic key — and its id is recorded in `uploads`. The Files API is
//! workspace-wide, so that table is the allowlist: `attachments` on a session
//! create or message must name ids this control plane uploaded, never any
//! other file in the workspace. Each attachment is mounted read-only in the
//! session's sandbox, and images and PDFs small enough are also put in the
//! message itself, so the model sees them without opening a tool.

use super::AppState;
use crate::anthropic::types::{ContentBlock, FileSource, SessionResource};
use crate::db::UploadRow;
use crate::error::{Error, Result};
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Multipart, Path, State};
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::Value;

/// Per file. The Files API takes 500 MB; this is a chat attachment, and the
/// body is held in memory on its way through.
pub const MAX_UPLOAD_BYTES: usize = 32 * 1024 * 1024;
/// Multipart framing on top of the file.
const BODY_LIMIT: usize = MAX_UPLOAD_BYTES + 64 * 1024;
pub const MAX_ATTACHMENTS: usize = 10;
/// Images and PDFs at or under this go into the message as blocks as well as
/// the sandbox. The block stays in the session's history and is re-read on
/// every turn, so a big one would cost on every turn — and a document the
/// model rejects (too many pages) would fail every turn after it.
pub const INLINE_MAX_BYTES: u64 = 5 * 1024 * 1024;
const INLINE_IMAGE_TYPES: &[&str] = &["image/jpeg", "image/png", "image/gif", "image/webp"];
const FILENAME_MAX_CHARS: usize = 100;
/// The task when a message is only attachments.
pub const DEFAULT_TASK: &str = "Take a look at the attached file(s).";

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/files",
            post(upload).route_layer(DefaultBodyLimit::max(BODY_LIMIT)),
        )
        .route("/files/{id}/content", get(content))
        .route("/sessions/{id}/files", get(session_files))
}

/// `POST /files`, multipart with one `file` part. Answers
/// `{file_id, filename, mime_type, size_bytes}`.
pub async fn upload(
    State(state): State<AppState>,
    mut form: Multipart,
) -> Result<(StatusCode, Json<UploadRow>)> {
    let bad = |e: axum::extract::multipart::MultipartError| {
        if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
            Error::InvalidRequest(format!(
                "file is larger than {} MB",
                MAX_UPLOAD_BYTES / 1024 / 1024
            ))
        } else {
            Error::InvalidRequest(format!("multipart: {}", e.body_text()))
        }
    };
    while let Some(field) = form.next_field().await.map_err(bad)? {
        if field.name() != Some("file") {
            continue;
        }
        let filename = clean_filename(field.file_name().unwrap_or(""));
        let mime = field
            .content_type()
            .map(|m| m.trim().to_ascii_lowercase())
            .filter(|m| plausible_mime(m) && m != "application/octet-stream")
            .unwrap_or_else(|| mime_from_name(&filename).to_owned());
        let bytes = field.bytes().await.map_err(bad)?;
        if bytes.is_empty() {
            return Err(Error::InvalidRequest("file is empty".into()));
        }
        if bytes.len() > MAX_UPLOAD_BYTES {
            return Err(Error::InvalidRequest(format!(
                "file is larger than {} MB",
                MAX_UPLOAD_BYTES / 1024 / 1024
            )));
        }
        let size = bytes.len();
        let obj = state
            .api
            .upload_file(&filename, Some(&mime), bytes.to_vec())
            .await?;
        let row = UploadRow {
            file_id: obj.id,
            filename,
            // The Files API's own detection when it has one.
            mime_type: if obj.mime_type.is_empty() {
                mime
            } else {
                obj.mime_type
            },
            size_bytes: if obj.size_bytes == 0 {
                size as u64
            } else {
                obj.size_bytes
            },
        };
        state.db.insert_upload(&row)?;
        tracing::info!(file = %row.file_id, mime = %row.mime_type, bytes = row.size_bytes, "file uploaded");
        return Ok((StatusCode::CREATED, Json(row)));
    }
    Err(Error::InvalidRequest(
        "expected a multipart part named `file`".into(),
    ))
}

/// `GET /sessions/{id}/files`: the Files API's list for that session, as is.
/// Agent outputs are the entries with `downloadable: true`.
pub async fn session_files(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    if !valid_id(&id) {
        return Err(Error::InvalidRequest(
            "session id has unexpected characters".into(),
        ));
    }
    Ok(Json(state.api.session_files_raw(&id).await?))
}

/// `GET /files/{id}/content`: streams a downloadable file, named.
pub async fn content(State(state): State<AppState>, Path(id): Path<String>) -> Result<Response> {
    if !valid_file_id(&id) {
        return Err(Error::InvalidRequest(
            "file id has unexpected characters".into(),
        ));
    }
    let meta = state.api.file_meta(&id).await?;
    let upstream = state.api.file_content(&id).await?;
    let ctype = upstream
        .headers()
        .get(header::CONTENT_TYPE)
        .cloned()
        .unwrap_or_else(|| header::HeaderValue::from_static("application/octet-stream"));
    let name = clean_filename(&meta.filename);
    Response::builder()
        .header(header::CONTENT_TYPE, ctype)
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{name}\""),
        )
        .body(Body::from_stream(upstream.bytes_stream()))
        .map_err(|e| Error::InvalidRequest(format!("response: {e}")))
}

/// `type/subtype`, no parameters, nothing a header could be split on.
fn plausible_mime(m: &str) -> bool {
    let mut parts = m.splitn(2, '/');
    let ok = |p: Option<&str>| {
        p.is_some_and(|p| {
            !p.is_empty()
                && p.len() <= 64
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"+-.".contains(&b))
        })
    };
    ok(parts.next()) && ok(parts.next())
}

/// When the browser sent no useful type: the common attachment kinds by
/// extension. The Files API detects the rest.
fn mime_from_name(name: &str) -> &'static str {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        "txt" | "log" => "text/plain",
        "md" => "text/markdown",
        "csv" => "text/csv",
        "json" => "application/json",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

pub fn valid_file_id(id: &str) -> bool {
    id.starts_with("file_") && id.len() > 5 && valid_id(id)
}

/// A name that is safe as a Files API filename, a sandbox path segment and a
/// `Content-Disposition` value: `[A-Za-z0-9._-]`, anything else becomes `_`,
/// at most 100 characters with the extension kept, never hidden or empty.
pub fn clean_filename(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let mut s: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    while s.contains("__") {
        s = s.replace("__", "_");
    }
    let s = s.trim_matches(|c| c == '.' || c == '_').to_owned();
    if s.is_empty() {
        return "upload".into();
    }
    if s.chars().count() <= FILENAME_MAX_CHARS {
        return s;
    }
    let (stem, ext) = match s.rfind('.') {
        Some(i) if s.len() - i <= 10 => (&s[..i], &s[i..]),
        _ => (s.as_str(), ""),
    };
    let keep = FILENAME_MAX_CHARS - ext.chars().count();
    format!("{}{ext}", stem.chars().take(keep).collect::<String>())
}

/// A mount path for `filename` that none of `taken` has: `/name.ext`, then
/// `/name-2.ext`, `/name-3.ext`, ….
pub fn free_mount_path(filename: &str, taken: &[String]) -> String {
    let first = format!("/{filename}");
    if !taken.contains(&first) {
        return first;
    }
    let (stem, ext) = match filename.rfind('.') {
        Some(i) if i > 0 => (&filename[..i], &filename[i..]),
        _ => (filename, ""),
    };
    (2..)
        .map(|n| format!("/{stem}-{n}{ext}"))
        .find(|p| !taken.contains(p))
        .expect("unbounded")
}

/// One attachment, resolved: the upload and where it sits in the sandbox.
#[derive(Debug, Clone, PartialEq)]
pub struct Attached {
    pub upload: UploadRow,
    pub mount_path: String,
    /// False when this session already had it mounted.
    pub new: bool,
}

impl Attached {
    pub fn sandbox_path(&self) -> String {
        format!("/mnt/session/uploads{}", self.mount_path)
    }
}

/// Look `ids` up in the allowlist and give each a mount path in `session`
/// (`None` while it is being created: nothing mounted yet). Duplicates in
/// `ids` collapse to one.
pub fn resolve(state: &AppState, session: Option<&str>, ids: &[String]) -> Result<Vec<Attached>> {
    if ids.len() > MAX_ATTACHMENTS {
        return Err(Error::InvalidRequest(format!(
            "at most {MAX_ATTACHMENTS} attachments per message"
        )));
    }
    let mut taken = match session {
        Some(s) => state.db.upload_mount_paths(s)?,
        None => vec![],
    };
    let mut out: Vec<Attached> = Vec::new();
    for id in ids {
        if !valid_file_id(id) {
            return Err(Error::InvalidRequest(format!(
                "attachment `{id}` is not a file id"
            )));
        }
        if out.iter().any(|a| &a.upload.file_id == id) {
            continue;
        }
        let upload = state.db.upload(id)?.ok_or_else(|| {
            Error::InvalidRequest(format!(
                "attachment `{id}` was not uploaded through this control plane"
            ))
        })?;
        if let Some(existing) = session
            .map(|s| state.db.upload_mount(s, id))
            .transpose()?
            .flatten()
        {
            out.push(Attached {
                upload,
                mount_path: existing,
                new: false,
            });
            continue;
        }
        let mount_path = free_mount_path(&upload.filename, &taken);
        taken.push(mount_path.clone());
        out.push(Attached {
            upload,
            mount_path,
            new: true,
        });
    }
    Ok(out)
}

pub fn resource(a: &Attached) -> SessionResource {
    SessionResource::File {
        file_id: a.upload.file_id.clone(),
        mount_path: a.mount_path.clone(),
    }
}

/// The `user.message` content for `task` plus `attached`: the text, then an
/// image block per small image and a document block per small PDF, then one
/// line per file saying where it is — which is also how an agent learns the
/// id to pass a file on to another session.
pub fn message_blocks(task: &str, attached: &[Attached]) -> Vec<ContentBlock> {
    let mut blocks = vec![ContentBlock::Text {
        text: task.to_owned(),
    }];
    if attached.is_empty() {
        return blocks;
    }
    let mut lines = vec!["Attached files (read-only copies in your sandbox):".to_owned()];
    for a in attached {
        let u = &a.upload;
        let small = u.size_bytes <= INLINE_MAX_BYTES;
        if small && INLINE_IMAGE_TYPES.contains(&u.mime_type.as_str()) {
            blocks.push(ContentBlock::Image {
                source: FileSource::file(&u.file_id),
            });
        } else if small && u.mime_type == "application/pdf" {
            blocks.push(ContentBlock::Document {
                source: FileSource::file(&u.file_id),
                title: Some(u.filename.clone()),
            });
        }
        lines.push(format!(
            "- {} ({}, {}; file_id {})",
            a.sandbox_path(),
            u.mime_type,
            human_size(u.size_bytes),
            u.file_id
        ));
    }
    blocks.push(ContentBlock::Text {
        text: lines.join("\n"),
    });
    blocks
}

fn human_size(n: u64) -> String {
    match n {
        n if n >= 1024 * 1024 => format!("{:.1} MB", n as f64 / 1024.0 / 1024.0),
        n if n >= 1024 => format!("{} KB", n.div_ceil(1024)),
        n => format!("{n} bytes"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn up(id: &str, name: &str, mime: &str, size: u64) -> Attached {
        Attached {
            upload: UploadRow {
                file_id: id.into(),
                filename: name.into(),
                mime_type: mime.into(),
                size_bytes: size,
            },
            mount_path: format!("/{name}"),
            new: true,
        }
    }

    #[test]
    fn filenames_are_cleaned() {
        assert_eq!(clean_filename("logo.png"), "logo.png");
        assert_eq!(clean_filename("My Logo (final).PNG"), "My_Logo_final_.PNG");
        assert_eq!(clean_filename("C:\\Users\\x\\a b.pdf"), "a_b.pdf");
        assert_eq!(clean_filename("../../etc/passwd"), "passwd");
        assert_eq!(clean_filename(".env"), "env");
        assert_eq!(clean_filename("日本.txt"), "txt");
        assert_eq!(clean_filename(""), "upload");
        assert_eq!(clean_filename("..."), "upload");
        let long = format!("{}.jpeg", "a".repeat(200));
        let c = clean_filename(&long);
        assert_eq!(c.chars().count(), FILENAME_MAX_CHARS);
        assert!(c.ends_with(".jpeg"));
    }

    #[test]
    fn mount_paths_never_clash() {
        assert_eq!(free_mount_path("a.png", &[]), "/a.png");
        let taken = vec!["/a.png".to_owned(), "/a-2.png".to_owned()];
        assert_eq!(free_mount_path("a.png", &taken), "/a-3.png");
        assert_eq!(free_mount_path("README", &["/README".into()]), "/README-2");
    }

    #[test]
    fn mime_types_are_checked_or_guessed() {
        assert!(plausible_mime("image/png"));
        assert!(plausible_mime(
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        ));
        assert!(!plausible_mime("text/plain; charset=utf-8"));
        assert!(!plausible_mime("image"));
        assert!(!plausible_mime("a/b\r\nx: y"));
        assert_eq!(mime_from_name("Shot.PNG"), "image/png");
        assert_eq!(mime_from_name("noext"), "application/octet-stream");
    }

    #[test]
    fn file_ids_are_checked() {
        assert!(valid_file_id("file_011CNha8iCJcU1wXNR6q4V8w"));
        assert!(!valid_file_id("file_"));
        assert!(!valid_file_id("sesn_1"));
        assert!(!valid_file_id("file_../x"));
    }

    #[test]
    fn no_attachments_is_a_plain_text_message() {
        assert_eq!(
            message_blocks("hi", &[]),
            vec![ContentBlock::Text { text: "hi".into() }]
        );
    }

    #[test]
    fn small_images_and_pdfs_go_inline_everything_is_listed() {
        let big = INLINE_MAX_BYTES + 1;
        let blocks = message_blocks(
            "look",
            &[
                up("file_img", "shot.png", "image/png", 2048),
                up("file_pdf", "contract.pdf", "application/pdf", 90_000),
                up("file_csv", "rows.csv", "text/csv", 500),
                up("file_bigimg", "huge.jpg", "image/jpeg", big),
                up("file_svg", "icon.svg", "image/svg+xml", 100),
            ],
        );
        assert_eq!(blocks.len(), 4, "{blocks:?}");
        assert_eq!(
            blocks[0],
            ContentBlock::Text {
                text: "look".into()
            }
        );
        assert_eq!(
            blocks[1],
            ContentBlock::Image {
                source: FileSource::file("file_img")
            }
        );
        assert_eq!(
            blocks[2],
            ContentBlock::Document {
                source: FileSource::file("file_pdf"),
                title: Some("contract.pdf".into())
            }
        );
        let ContentBlock::Text { text } = &blocks[3] else {
            panic!("{blocks:?}")
        };
        assert_eq!(text.lines().count(), 6, "{text}");
        assert!(
            text.contains("/mnt/session/uploads/rows.csv (text/csv, 500 bytes; file_id file_csv)")
        );
        assert!(text
            .contains("/mnt/session/uploads/huge.jpg (image/jpeg, 5.0 MB; file_id file_bigimg)"));
    }

    #[test]
    fn blocks_serialize_to_the_documented_shapes() {
        let v = serde_json::to_value(message_blocks(
            "t",
            &[up("file_1", "a.pdf", "application/pdf", 1)],
        ))
        .unwrap();
        assert_eq!(v[0], serde_json::json!({"type": "text", "text": "t"}));
        assert_eq!(
            v[1],
            serde_json::json!({"type": "document", "source": {"type": "file", "file_id": "file_1"}, "title": "a.pdf"})
        );
        let r =
            serde_json::to_value(resource(&up("file_1", "a.pdf", "application/pdf", 1))).unwrap();
        assert_eq!(
            r,
            serde_json::json!({"type": "file", "file_id": "file_1", "mount_path": "/a.pdf"})
        );
    }

    // ---- the routes end to end, against a stand-in for Anthropic

    type Seen = std::sync::Arc<std::sync::Mutex<Vec<(String, Value)>>>;

    /// Records `(path, json body)` for every JSON call; answers uploads with
    /// a file object, session create with a session, everything else `{}`.
    async fn fake_anthropic(seen: Seen) -> String {
        use axum::extract::{Request, State as S};
        async fn any(S(seen): S<Seen>, req: Request) -> Json<Value> {
            let path = req.uri().path().to_owned();
            let ctype = req
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_owned();
            let bytes = axum::body::to_bytes(req.into_body(), usize::MAX)
                .await
                .unwrap();
            if path == "/v1/files" {
                assert!(ctype.starts_with("multipart/form-data"), "{ctype}");
                let body = String::from_utf8_lossy(&bytes);
                assert!(body.contains("filename=\"My_Shot.png\""), "{body}");
                seen.lock().unwrap().push((path, Value::Null));
                return Json(serde_json::json!({
                    "id": format!("file_{}", seen.lock().unwrap().len()),
                    "type": "file", "filename": "My_Shot.png",
                    "mime_type": "image/png", "size_bytes": bytes.len()
                }));
            }
            let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            seen.lock().unwrap().push((path.clone(), body));
            if path == "/v1/sessions" {
                return Json(serde_json::json!({"id": "sesn_1", "status": "idle"}));
            }
            Json(serde_json::json!({}))
        }
        let app = Router::new().fallback(any).with_state(seen);
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        format!("http://{addr}")
    }

    async fn control_plane(anthropic: String) -> (String, crate::db::Db) {
        let db = crate::db::Db::in_memory().unwrap();
        db.upsert_environment("jarvis-lab", "cloud", Some("env_1"))
            .unwrap();
        db.upsert_agent(&crate::db::AgentRow {
            slug: "jarvis".into(),
            agent_id: "agent_1".into(),
            agent_version: 6,
            definition_sha256: String::new(),
            max_list_cost_cents: "200".parse().unwrap(),
            effort: "medium".into(),
            default_environment: "jarvis-lab".into(),
            synced_at: String::new(),
        })
        .unwrap();
        let state = AppState {
            api: crate::anthropic::Client::new(anthropic, "sk-test").unwrap(),
            db: db.clone(),
            signing_key: crate::webhook::signature::SigningKey::parse("whsec_dGVzdA==").unwrap(),
            seen_events: std::sync::Arc::new(crate::webhook::SeenEvents::new(10)),
            control_plane_token: std::sync::Arc::new("t".into()),
            console_workspace: std::sync::Arc::new("default".into()),
            mcp_fleet_vault_id: std::sync::Arc::new(None),
            inference: std::sync::Arc::new(None),
        };
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, super::super::router(state)).await.unwrap() });
        (format!("http://{addr}"), db)
    }

    async fn upload_png(http: &reqwest::Client, cp: &str) -> Value {
        let part = reqwest::multipart::Part::bytes(vec![0x89, b'P', b'N', b'G'])
            .file_name("My Shot.png")
            .mime_str("image/png")
            .unwrap();
        let res = http
            .post(format!("{cp}/files"))
            .bearer_auth("t")
            .multipart(reqwest::multipart::Form::new().part("file", part))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 201);
        res.json().await.unwrap()
    }

    #[tokio::test]
    async fn upload_then_attach_at_create_and_on_a_follow_up() {
        let seen: Seen = Default::default();
        let (cp, db) = control_plane(fake_anthropic(seen.clone()).await).await;
        let http = reqwest::Client::new();

        let first = upload_png(&http, &cp).await;
        assert_eq!(first["file_id"], "file_1");
        assert_eq!(first["filename"], "My_Shot.png");
        assert_eq!(first["mime_type"], "image/png");
        assert!(db.upload("file_1").unwrap().is_some());

        // Not ours: refused before anything reaches Anthropic.
        let res = http
            .post(format!("{cp}/sessions"))
            .bearer_auth("t")
            .json(&serde_json::json!({"agent_slug": "jarvis", "task": "x", "attachments": ["file_elsewhere"]}))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 400);

        // Create with only an attachment: the stock task, the file mounted
        // and shown.
        let res = http
            .post(format!("{cp}/sessions"))
            .bearer_auth("t")
            .json(&serde_json::json!({"agent_slug": "jarvis", "task": "  ", "attachments": ["file_1"]}))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 201, "{}", res.text().await.unwrap());
        let create = seen
            .lock()
            .unwrap()
            .iter()
            .find(|(p, _)| p == "/v1/sessions")
            .unwrap()
            .1
            .clone();
        assert_eq!(
            create["resources"],
            serde_json::json!([{"type": "file", "file_id": "file_1", "mount_path": "/My_Shot.png"}])
        );
        let content = &create["initial_events"][0]["content"];
        assert_eq!(content[0]["text"], DEFAULT_TASK);
        assert_eq!(content[1]["type"], "image");
        assert!(content[2]["text"]
            .as_str()
            .unwrap()
            .contains("/mnt/session/uploads/My_Shot.png"));

        // A second upload with the same name, sent with the first again: only
        // the new one is added, at a path of its own.
        let second = upload_png(&http, &cp).await;
        let id2 = second["file_id"].as_str().unwrap().to_owned();
        let res = http
            .post(format!("{cp}/sessions/sesn_1/events"))
            .bearer_auth("t")
            .json(&serde_json::json!({"task": "and these", "attachments": ["file_1", id2]}))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 200, "{}", res.text().await.unwrap());
        let calls = seen.lock().unwrap().clone();
        let added: Vec<&Value> = calls
            .iter()
            .filter(|(p, _)| p == "/v1/sessions/sesn_1/resources")
            .map(|(_, b)| b)
            .collect();
        assert_eq!(
            added,
            vec![
                &serde_json::json!({"type": "file", "file_id": id2, "mount_path": "/My_Shot-2.png"})
            ]
        );
        let (_, sent) = calls
            .iter()
            .rfind(|(p, _)| p == "/v1/sessions/sesn_1/events")
            .unwrap();
        let text = sent["events"][0]["content"][3]["text"].as_str().unwrap();
        assert!(
            text.contains("/mnt/session/uploads/My_Shot.png ") && text.contains("/My_Shot-2.png "),
            "{text}"
        );
    }
}
