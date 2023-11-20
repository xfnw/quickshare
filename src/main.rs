use std::{fs::File, include_str, io::prelude::*, net::SocketAddr};
use structopt::StructOpt;

use axum::{
    extract::{DefaultBodyLimit, Multipart},
    http::StatusCode,
    response::Html,
    routing::{get, post},
    Router,
};

#[derive(Debug, StructOpt)]
#[structopt(name = "quickshare", about = "quickly spin up a file upload form")]
struct Opt {
    #[structopt(short, env = "BIND", default_value = "[::]:3000")]
    bindhost: SocketAddr,
    #[structopt(short, help = "max upload size in MiB", default_value = "1024")]
    limit: usize,
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

async fn upload(mut multipart: Multipart) -> Result<&'static str, (StatusCode, String)> {
    while let Some(mut field) = unwrap_or_bad!(multipart.next_field().await) {
        if Some("file") != field.name() {
            continue;
        }

        let name = format!(
            "quickshare_{}",
            field.file_name().unwrap_or("untitled").replace('/', "")
        );

        // TODO: consider changing this to File::create_new once stabilized
        // https://github.com/rust-lang/rust/issues/105135
        let mut file = unwrap_or_bad!(File::options().write(true).create_new(true).open(&name));
        while let Some(chunk) = unwrap_or_bad!(field.chunk().await) {
            unwrap_or_bad!(file.write_all(&chunk));
        }

        eprintln!("received {}", name);
        return Ok("uploaded~");
    }

    Ok("you did not send a file? less work for me i guess")
}

#[tokio::main]
async fn main() {
    let opt = Opt::from_args();
    let app = Router::new()
        .route("/", get(root))
        .route("/", post(upload))
        .layer(DefaultBodyLimit::max(opt.limit * 1048576));

    eprintln!("listening on {}", opt.bindhost);
    axum::Server::bind(&opt.bindhost)
        .serve(app.into_make_service())
        .await
        .unwrap();
}
