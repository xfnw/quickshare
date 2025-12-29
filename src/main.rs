#![deny(clippy::pedantic)]
#![allow(clippy::unnecessary_debug_formatting)]

use argh::FromArgs;
use axum::{
    body::{Body as AxumBody, HttpBody},
    extract::{DefaultBodyLimit, Multipart, Path, State},
    http::{header::HeaderMap, Response, StatusCode},
    response::{Html, IntoResponse},
    routing::{get, get_service, post, put},
    Router,
};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    include_str,
    io::prelude::*,
    net::SocketAddr,
    path::{Component, Path as StdPath, PathBuf},
    pin::Pin,
    sync::Arc,
};
use tokio::{
    net::TcpListener,
    sync::{mpsc, oneshot, Mutex, MutexGuard},
};
use tokio_stream::StreamExt;
use tower_http::services::ServeDir;

/// quickly spin up a file upload form
#[derive(Debug, FromArgs)]
#[argh(help_triggers("-h", "--help"))]
#[allow(clippy::struct_excessive_bools)]
struct Opt {
    /// socket address to bind
    #[argh(option, short = 'b', default = "\"[::]:3000\".parse().unwrap()")]
    bind: SocketAddr,
    /// max form upload size in MiB (default: 10)
    #[argh(option, short = 'l', default = "10")]
    limit: usize,
    /// allow access to contents of current directory
    #[argh(switch, short = 's')]
    serve: bool,
    /// turn off upload form
    #[argh(switch)]
    no_upload: bool,
    /// turn off uploading via put
    #[argh(switch)]
    no_put: bool,
    /// turn off piping
    #[argh(switch)]
    no_pipe: bool,
}

struct AppState {
    pipes: Mutex<BTreeMap<String, Pipe>>,
}

struct Pipe {
    sender: Arc<mpsc::Sender<PipeBody>>,
    receiver: Arc<Mutex<mpsc::Receiver<PipeBody>>>,
}

impl Pipe {
    fn new() -> Self {
        let (sender, receiver) = mpsc::channel(1);
        Self {
            sender: Arc::new(sender),
            receiver: Arc::new(Mutex::new(receiver)),
        }
    }
}

struct DropSender {
    sender: Option<oneshot::Sender<()>>,
}

impl Drop for DropSender {
    fn drop(&mut self) {
        if let Some(sender) = self.sender.take() {
            let _ = sender.send(());
        }
    }
}

/// a wrapper around [`AxumBody`] that sends to a oneshot channel
/// when dropped
struct PipeBody {
    inner: AxumBody,
    _on_drop: DropSender,
}

impl HttpBody for PipeBody {
    type Data = axum::body::Bytes;
    type Error = axum::Error;

    #[inline]
    fn poll_frame(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        Pin::new(&mut self.inner).poll_frame(cx)
    }

    #[inline]
    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    #[inline]
    fn size_hint(&self) -> http_body::SizeHint {
        self.inner.size_hint()
    }
}

impl IntoResponse for PipeBody {
    fn into_response(self) -> axum::response::Response {
        Response::new(AxumBody::new(self))
    }
}

async fn root() -> Html<&'static str> {
    Html(include_str!("form.html"))
}

macro_rules! unwrap_or_bad {
    ($ex:expr) => {
        match $ex {
            Ok(v) => v,
            Err(e) => {
                eprintln!("error {:?}", e);
                return Err((StatusCode::BAD_REQUEST, format!("{e}\n")));
            }
        }
    };
}

async fn upload(
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<String, (StatusCode, String)> {
    while let Some(mut field) = unwrap_or_bad!(multipart.next_field().await) {
        if Some("file") != field.name() {
            continue;
        }

        let name = sanitize_path(field.file_name().unwrap_or("untitled"));
        if let Some(parent) = name.parent() {
            unwrap_or_bad!(fs::create_dir_all(parent));
        }

        let mut file = unwrap_or_bad!(File::create_new(&name));
        while let Some(chunk) = unwrap_or_bad!(field.chunk().await) {
            unwrap_or_bad!(file.write_all(&chunk));
        }

        eprintln!("received {name:?}");

        let proto = headers
            .get("x-forwarded-proto")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("http");
        let host = headers
            .get("host")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("localhost");
        return Ok(format!("{proto}://{host}/{}\n", name.display()));
    }

    Err((StatusCode::BAD_REQUEST, "no file? 😳".to_string()))
}

async fn upload_put(
    headers: HeaderMap,
    Path(name): Path<PathBuf>,
    body: AxumBody,
) -> Result<String, (StatusCode, String)> {
    let name = sanitize_path(&name);
    if let Some(parent) = name.parent() {
        unwrap_or_bad!(fs::create_dir_all(parent));
    }

    let mut file = unwrap_or_bad!(File::create_new(&name));
    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = unwrap_or_bad!(chunk);
        unwrap_or_bad!(file.write_all(&chunk));
    }

    eprintln!("received {name:?}");

    let proto = headers
        .get("x-forwarded-proto")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("http");
    let host = headers
        .get("host")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost");
    Ok(format!("{proto}://{host}/{}\n", name.display()))
}

fn sanitize_path(path: impl AsRef<StdPath>) -> PathBuf {
    let mut out = PathBuf::new();

    for component in path.as_ref().components() {
        match component {
            Component::Prefix(_) | Component::RootDir | Component::CurDir => (),
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(c) => {
                out.push(c);
            }
        }
    }

    out
}

fn pipecleaner(pipes: &mut MutexGuard<BTreeMap<String, Pipe>>) {
    // kinda wasteful to do this every time, but manually
    // figuring out when to remove stuff is complicated
    pipes.retain(|_, pipe| {
        Arc::strong_count(&pipe.sender) > 1 || Arc::strong_count(&pipe.receiver) > 1
    });
}

async fn recv_pipe(
    Path(name): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<PipeBody, (StatusCode, &'static str)> {
    let receiver = {
        let mut pipes = state.pipes.lock().await;
        pipecleaner(&mut pipes);
        pipes.entry(name).or_insert_with(Pipe::new).receiver.clone()
    };
    let mut receiver = receiver.lock().await;

    receiver
        .recv()
        .await
        .ok_or((StatusCode::INTERNAL_SERVER_ERROR, "the channel closed???\n"))
}

async fn send_pipe(
    Path(name): Path<String>,
    State(state): State<Arc<AppState>>,
    body: AxumBody,
) -> Result<(), (StatusCode, String)> {
    let pipe_sender = {
        let mut pipes = state.pipes.lock().await;
        pipecleaner(&mut pipes);
        pipes.entry(name).or_insert_with(Pipe::new).sender.clone()
    };

    let (drop_sender, finished) = oneshot::channel();
    let body = PipeBody {
        inner: body,
        _on_drop: DropSender {
            sender: Some(drop_sender),
        },
    };

    unwrap_or_bad!(pipe_sender.send(body).await);

    unwrap_or_bad!(finished.await);

    Ok(())
}

#[tokio::main]
async fn main() {
    let opt: Opt = argh::from_env();
    let state = Arc::new(AppState {
        pipes: Mutex::new(BTreeMap::new()),
    });
    let app = Router::new();

    let app = if opt.no_upload {
        app
    } else {
        app.route("/", get(root))
            .route("/", post(upload))
            .layer(DefaultBodyLimit::max(opt.limit * 1_048_576))
    };

    let app = if opt.no_put {
        app
    } else {
        app.route("/{*name}", put(upload_put))
    };

    let app = if opt.no_pipe {
        app
    } else {
        app.route("/pipe/{*name}", get(recv_pipe))
            .route("/pipe/{*name}", post(send_pipe))
    };

    let app = app.with_state(state);

    let app = if opt.serve {
        app.route("/{*name}", get_service(ServeDir::new(".")))
    } else {
        app
    };

    let listen = TcpListener::bind(&opt.bind).await.unwrap();
    eprintln!("listening on {}", listen.local_addr().unwrap());
    axum::serve(listen, app.into_make_service()).await.unwrap();
}
