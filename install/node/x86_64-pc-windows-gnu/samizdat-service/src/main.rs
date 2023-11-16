#[cfg(target_os = "windows")]
mod windows {
    use std::ffi::OsString;
    use std::process::Command;

    use windows_service::define_windows_service;
    use windows_service::service_dispatcher;

    define_windows_service!(ffi_service_main, my_service_main);

    fn my_service_main(arguments: Vec<OsString>) {
        // The entry point where execution will start on a background thread after a call to
        // `service_dispatcher::start` from `main`.
        let mut command = Command::new("samizdat-node.exe");
        command.args(arguments.clone());

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
    }
}

#[cfg(target_os = "windows")]
fn main() -> Result<(), windows_service::Error> {
    // Register generated `ffi_service_main` with the system and start the service, blocking
    // this thread until the service is stopped.
    service_dispatcher::start("myservice", ffi_service_main)?;
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn main() -> Result<(), ()> {
    panic!("This program only makes sense in Windows");
}
