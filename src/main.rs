use std::{fs::File, include_str, io::prelude::*, net::SocketAddr};
use structopt::StructOpt;

use axum::{
    extract::Multipart,
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
}

async fn root() -> Html<&'static str> {
    Html(include_str!("form.html"))
}

async fn upload(mut multipart: Multipart) -> Result<&'static str, (StatusCode, String)> {
    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
    {
        if Some("file") != field.name() {
            continue;
        }

        let name = format!(
            "quickshare_{}",
            field
                .file_name()
                .unwrap_or_else(|| "no-name")
                .replace('/', "")
        );

        eprintln!("received {}", name);
        let mut file = File::create(name).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
        while let Some(chunk) = field
            .chunk()
            .await
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
        {
            println!("i got a chunk!");
            file.write_all(&chunk);
        }
    }

    Ok("you did not send a file? less work for me i guess")
}

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
