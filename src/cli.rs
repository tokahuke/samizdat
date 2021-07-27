use structopt::StructOpt;

#[derive(StructOpt)]
pub struct Cli {
    #[structopt(env, default_value = "data/db")]
    pub db_path: String,
}
