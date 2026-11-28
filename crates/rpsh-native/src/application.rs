//! Construct a native application command after PowerShell has resolved its path.
use std::borrow::Cow;
use std::io;
use std::path::Path;
use std::process::Command;

/// Apply the shell's overrides to the inherited environment. The engine may
/// attach pipes or a terminal after constructing the command.
pub fn application_command<'a>(
    program: &Path,
    arguments: &[String],
    environment: impl IntoIterator<Item = (&'a str, Option<Cow<'a, str>>)>,
) -> io::Result<Command> {
    let mut command = Command::new(program);
    configure(&mut command, arguments, environment);
    #[cfg(target_os = "macos")]
    exact_exec::configure(&mut command)?;
    Ok(command)
}

fn configure<'a>(
    command: &mut Command,
    arguments: &[String],
    environment: impl IntoIterator<Item = (&'a str, Option<Cow<'a, str>>)>,
) {
    command.args(arguments);
    for (key, value) in environment {
        match value {
            Some(value) => command.env(key, value.as_ref()),
            None => command.env_remove(key),
        };
    }
}

/// Replace the calling process without shell fallback, using the basename as
/// argv[0] as Switch-Process does. Success never returns.
#[cfg(unix)]
pub fn replace_process<'a>(
    program: &Path,
    arguments: &[String],
    environment: impl IntoIterator<Item = (&'a str, Option<Cow<'a, str>>)>,
) -> io::Error {
    let mut command = Command::new(program);
    configure(&mut command, arguments, environment);
    let argv0 = program.file_name().unwrap_or(program.as_os_str());
    match exact_exec::Image::new(&command, argv0) {
        Ok(image) => image.replace(),
        Err(error) => error,
    }
}

// Darwin's posix_spawnp and execvp both start /bin/sh on ENOEXEC, even for
// absolute paths. PowerShell uses the kernel's result for an application.
// Use the same exact syscall for Unix process replacement.
#[cfg(unix)]
mod exact_exec {
    #![allow(unsafe_code)]
    use std::collections::BTreeMap;
    use std::ffi::{CString, OsStr, OsString, c_char};
    use std::io;
    use std::os::unix::ffi::OsStrExt;
    #[cfg(any(target_os = "macos", test))]
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    unsafe extern "C" {
        fn execve(
            path: *const c_char,
            argv: *const *const c_char,
            envp: *const *const c_char,
        ) -> i32;
    }

    /// execve borrows a null-terminated vector of pointers. The strings own
    /// their allocations; moving the owner cannot move the string buffers.
    struct Strings {
        _storage: Vec<CString>,
        pointers: Vec<*const c_char>,
    }

    // SAFETY: pointers refer only to immutable CString buffers owned by this
    // struct. All access after construction is read-only, including in execve.
    unsafe impl Send for Strings {}
    // SAFETY: sharing this immutable argument block does not share mutable data.
    unsafe impl Sync for Strings {}

    impl Strings {
        fn new(items: impl IntoIterator<Item = impl AsRef<OsStr>>) -> io::Result<Self> {
            let storage = items
                .into_iter()
                .map(|item| CString::new(item.as_ref().as_bytes()))
                .collect::<Result<Vec<_>, _>>()?;
            let pointers = storage
                .iter()
                .map(|item| item.as_ptr())
                .chain([std::ptr::null()])
                .collect();
            Ok(Self {
                _storage: storage,
                pointers,
            })
        }

        fn as_ptr(&self) -> *const *const c_char {
            self.pointers.as_ptr()
        }
    }

    pub(super) struct Image {
        program: CString,
        argv: Strings,
        envp: Strings,
    }

    impl Image {
        pub(super) fn new(command: &Command, argv0: &OsStr) -> io::Result<Self> {
            let program = CString::new(command.get_program().as_bytes())?;
            let argv = Strings::new(std::iter::once(argv0).chain(command.get_args()))?;
            let mut environment: BTreeMap<OsString, OsString> = std::env::vars_os().collect();
            for (key, value) in command.get_envs() {
                match value {
                    Some(value) => {
                        environment.insert(key.into(), value.into());
                    }
                    None => {
                        environment.remove(key);
                    }
                }
            }
            let envp = Strings::new(environment.into_iter().map(|(mut key, value)| {
                key.push("=");
                key.push(value);
                key
            }))?;
            Ok(Self {
                program,
                argv,
                envp,
            })
        }

        pub(super) fn replace(&self) -> io::Error {
            // SAFETY: the null-terminated pointers borrow this immutable image.
            // Only the async-signal-safe execve and errno access occur here.
            unsafe {
                execve(
                    self.program.as_ptr(),
                    self.argv.as_ptr(),
                    self.envp.as_ptr(),
                );
            }
            io::Error::last_os_error()
        }
    }

    #[cfg(any(target_os = "macos", test))]
    pub(super) fn configure(command: &mut Command) -> io::Result<()> {
        let image = Image::new(command, command.get_program())?;
        // SAFETY: replace() only calls execve and reads errno after the fork.
        unsafe {
            command.pre_exec(move || Err(image.replace()));
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::configure;
        use std::fs;
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        use std::process::Command;

        #[test]
        fn preserves_arguments_environment_and_pipes() {
            let mut command = Command::new("/bin/sh");
            command.args([
                "-c",
                "printf '%s|%s|%s' \"$1\" \"$RPSH_CHILD_TEST\" \"${HOME-unset}\"",
                "sh",
                "a b",
            ]);
            command.env("RPSH_CHILD_TEST", "ü=1").env_remove("HOME");
            configure(&mut command).unwrap();
            let result = command.output().unwrap();
            assert!(result.status.success());
            assert_eq!(result.stdout, "a b|ü=1|unset".as_bytes());
        }

        #[test]
        fn an_unrecognized_executable_does_not_start_a_shell() {
            let path =
                std::env::temp_dir().join(format!("rpsh-exec-format-{}", std::process::id()));
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o700)
                .open(&path)
                .unwrap();
            file.write_all(b"exit 17\n").unwrap();
            drop(file);
            let mut command = Command::new(&path);
            configure(&mut command).unwrap();
            let result = command.status();
            fs::remove_file(path).unwrap();
            assert_eq!(result.unwrap_err().raw_os_error(), Some(8)); // ENOEXEC on Linux and Darwin.
        }
    }
}
