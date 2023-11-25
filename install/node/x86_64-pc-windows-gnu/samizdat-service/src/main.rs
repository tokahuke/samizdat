#[cfg(target_os = "windows")]
fn main() -> Result<(), windows_service::Error> {
    use std::ffi::OsString;
    use std::fs::File;
    use std::fs::File;
    use std::process::{Command, Stdio};

    use windows_service::define_windows_service;
    use windows_service::service_dispatcher;

    define_windows_service!(ffi_service_main, my_service_main);

    pub fn my_service_main(arguments: Vec<OsString>) {
        let maybe_create_file = |name: &str| {
            if let Ok(file) = File::create(name) {
                Stdio::from(file)
            } else {
                eprintln!("Could not create {name} stdio! Will inherit from the current process");
                Stdio::inherit()
            }
        };
        let stdout_log = maybe_create_file(r"C:\ProgramData\Samizdat\Node\stdout.log");
        let stderr_log = maybe_create_file(r"C:\ProgramData\Samizdat\Node\stderr.log");

        loop {
            // The entry point where execution will start on a background thread after a call to
            // `service_dispatcher::start` from `main`.
            let mut command = Command::new("samizdat-node.exe");
            command.args(arguments.clone());
            command.env("RUST_BACKTRACE", "1");
            command.stdout(stdout);
            command.stderr(stderr);

            match command.spawn() {
                Ok(mut child) => match child.wait() {
                    Ok(status) if status.success() => {
                        eprintln!(
                            "samizdat-node.exe {:?} exited with status {status}",
                            arguments
                        )
                    }
                    Ok(status) => {
                        eprintln!(
                            "samizdat-node.exe {:?} failed with status {status}",
                            arguments
                        )
                    }
                    Err(err) => {
                        eprintln!(
                            "Failed to wait for samizdat-node.exe {:?}: {err}",
                            arguments
                        );
                    }
                },
                Err(err) => {
                    eprintln!("Failed to invoke samizdat-node.exe {:?}: {err}", arguments);
                }
            }

            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }

    // Register generated `ffi_service_main` with the system and start the service, blocking
    // this thread until the service is stopped.
    service_dispatcher::start("SamizdatNode", ffi_service_main)?;
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn main() -> ! {
    panic!("This program only makes sense in Windows")
}
