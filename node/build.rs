use std::process::Command;

fn main() {
    let run_flatc = |name: &str| {
        Command::new("flatc")
            .arg("--rust")
            .arg(name)
            .current_dir("./src/flatbuffers")
            .spawn()
            .unwrap()
    };

    run_flatc("object.fbs");
}
