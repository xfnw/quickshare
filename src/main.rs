#![deny(clippy::pedantic)]

use argh::FromArgs;
use std::{
    collections::BTreeMap, fs::File, include_str, io::prelude::*, net::SocketAddr, sync::Arc,
};
use tokio::{
    net::TcpListener,
    sync::{
        mpsc::{channel, Receiver, Sender},
        Mutex, MutexGuard,
    },
};
use tower_http::services::ServeDir;

use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Multipart, Path, State},
    http::{header::HeaderMap, StatusCode},
    response::Html,
    routing::{get, post},
    Router,
};

/// quickly spin up a file upload form
#[derive(Debug, FromArgs)]
#[argh(help_triggers("-h", "--help"))]
#[allow(clippy::struct_excessive_bools)]
struct Opt {
    /// socket address to bind
    #[argh(option, short = 'b', default = "\"[::]:3000\".parse().unwrap()")]
    bind: SocketAddr,
    /// max upload size in MiB (default: 1024)
    #[argh(option, short = 'l', default = "1024")]
    limit: usize,
    /// allow access to contents of current directory
    #[argh(switch, short = 's')]
    serve: bool,
    /// omit the quickshare_ prefix added to filenames
    #[argh(switch, short = 'u')]
    unprefixed: bool,
    /// turn off uploading files
    #[argh(switch)]
    no_upload: bool,
    /// turn off piping
    #[argh(switch)]
    no_pipe: bool,
}

struct AppState {
    unprefixed: bool,
    pipes: Mutex<BTreeMap<String, Pipe>>,
}

struct Pipe {
    sender: Arc<Mutex<Sender<Body>>>,
    receiver: Arc<Mutex<Receiver<Body>>>,
}

impl Pipe {
    fn new() -> Self {
        let (sender, receiver) = channel(1);
        Self {
            sender: Arc::new(Mutex::new(sender)),
            receiver: Arc::new(Mutex::new(receiver)),
        }
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
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<String, (StatusCode, String)> {
    while let Some(mut field) = unwrap_or_bad!(multipart.next_field().await) {
        if Some("file") != field.name() {
            continue;
        }

        let name = field.file_name().unwrap_or("untitled").replace('/', "");
        let name = if state.unprefixed {
            name
        } else {
            format!("quickshare_{name}")
        };

        let mut file = unwrap_or_bad!(File::create_new(&name));
        while let Some(chunk) = unwrap_or_bad!(field.chunk().await) {
            unwrap_or_bad!(file.write_all(&chunk));
        }

        eprintln!("received {name}");

        let proto = headers
            .get("x-forwarded-proto")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("http");
        let host = headers
            .get("host")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("localhost");
        return Ok(format!("{proto}://{host}/{name}\n"));
    }

    Err((StatusCode::BAD_REQUEST, "no file? 😳".to_string()))
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
) -> Result<Body, (StatusCode, &'static str)> {
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
    body: Body,
) -> Result<(), (StatusCode, String)> {
    let sender = {
        let mut pipes = state.pipes.lock().await;
        pipecleaner(&mut pipes);
        pipes.entry(name).or_insert_with(Pipe::new).sender.clone()
    };
    let sender = sender.lock().await;

    unwrap_or_bad!(sender.send(body).await);
    // cursed way to pretend mpsc waits for the receiver,
    // instead of returning immediately when there is capacity,
    // since tokio does not allow having 0 capacity :(
    // this only works because sender is locked behind a Mutex
    unwrap_or_bad!(sender.reserve().await);

    Ok(())
}

#[tokio::main]
async fn main() {
    let opt: Opt = argh::from_env();
    let state = Arc::new(AppState {
        unprefixed: opt.unprefixed,
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

    let app = if opt.no_pipe {
        app
    } else {
        app.route("/pipe/{*name}", get(recv_pipe))
            .route("/pipe/{*name}", post(send_pipe))
    };

    let app = app.with_state(state);

    let app = if opt.serve {
        app.fallback_service(ServeDir::new("."))
    } else {
        app
    };

    let listen = TcpListener::bind(&opt.bind).await.unwrap();
    eprintln!("listening on {}", listen.local_addr().unwrap());
    axum::serve(listen, app.into_make_service()).await.unwrap();
}
