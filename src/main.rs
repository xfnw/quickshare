use structopt::StructOpt;

#[derive(Debug, StructOpt)]
#[structopt(name = "quickshare", about = "quickly spin up a file upload form")]
struct Opt {
    #[structopt(short, env = "PORT", default_value = "3000")]
    port: u16,
}

fn main() {
    let opt = Opt::from_args();
}
