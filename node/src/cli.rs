use structopt::StructOpt;

#[derive(StructOpt)]
pub struct Cli {
    #[structopt(env, long, default_value = "data/db")]
    pub db_path: String,
    #[structopt(env, long, default_value = "4510")]
    pub port: u16,
    #[structopt(env, long, default_value = "64000000")]
    pub max_content_size: usize,
}

static mut CLI: Option<Cli> = None;

pub fn init_cli() -> Result<(), crate::Error> {
    let cli = Cli::from_args();

    unsafe {
        CLI = Some(cli);
    }

    Ok(())
}

pub fn cli<'a>() -> &'a Cli {
    unsafe { CLI.as_ref().expect("cli not initialized") }
}
