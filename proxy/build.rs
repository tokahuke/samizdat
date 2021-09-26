fn main() {
    std::process::Command::new("./refresh-static.sh").spawn().unwrap().wait().unwrap();
}