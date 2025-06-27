#![deny(clippy::pedantic)]

use argh::FromArgs;
use std::{fs::File, include_str, io::prelude::*, net::SocketAddr, sync::Arc};
use tokio::net::TcpListener;
use tower_http::services::ServeDir;

use axum::{
    extract::{DefaultBodyLimit, Multipart, State},
    http::{header::HeaderMap, StatusCode},
    response::Html,
    routing::{get, post},
    Router,
};

/// quickly spin up a file upload form
#[derive(Debug, FromArgs)]
#[argh(help_triggers("-h", "--help"))]
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
}

struct AppState {
    unprefixed: bool,
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
                return Err((StatusCode::BAD_REQUEST, e.to_string()));
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

#[tokio::main]
async fn main() {
    let opt: Opt = argh::from_env();
    let state = Arc::new(AppState {
        unprefixed: opt.unprefixed,
    });
    let app = Router::new()
        .route("/", get(root))
        .route("/", post(upload))
        .layer(DefaultBodyLimit::max(opt.limit * 1_048_576))
        .with_state(state);

    let app = if opt.serve {
        app.fallback_service(ServeDir::new("."))
    } else {
        app
    };

    let listen = TcpListener::bind(&opt.bind).await.unwrap();
    eprintln!("listening on {}", listen.local_addr().unwrap());
    axum::serve(listen, app.into_make_service()).await.unwrap();
}
