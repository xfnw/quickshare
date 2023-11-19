use std::{include_str, net::SocketAddr};
use structopt::StructOpt;

use axum::{
    extract::Multipart,
    response::Html,
    routing::{get, post},
    Router,
};

#[derive(Debug, StructOpt)]
#[structopt(name = "quickshare", about = "quickly spin up a file upload form")]
struct Opt {
    #[structopt(short, env = "BIND", default_value = "[::]:3000")]
    bindhost: SocketAddr,
}

async fn root() -> Html<&'static str> {
    Html(include_str!("form.html"))
}

async fn upload(mut multipart: Multipart) {}

#[tokio::main]
async fn main() {
    let opt = Opt::from_args();
    let app = Router::new()
        .route("/", get(root))
        .route("/up", post(upload));

    eprintln!("listening on {}", opt.bindhost);
    axum::Server::bind(&opt.bindhost)
        .serve(app.into_make_service())
        .await
        .unwrap();
}
