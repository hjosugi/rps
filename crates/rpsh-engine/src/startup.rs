//! Opt-in process-wide first-use trace. The normal path never opens a file
//! or formats a message. In an embedded process the first session also
//! determines whether tracing is enabled for subsequent sessions.
use std::io::{self, Write};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

static ENABLED: OnceLock<bool> = OnceLock::new();
static SEEN: AtomicU32 = AtomicU32::new(0);

fn enabled() -> bool {
    *ENABLED.get_or_init(|| {
        std::env::var_os("RPSH_TRACE_INIT").is_some_and(|value| !value.is_empty() && value != "0")
    })
}

#[derive(Clone, Copy)]
pub(crate) enum Stage {
    Arguments,
    Session,
    Parse,
    Variables,
    Functions,
    Aliases,
    Cmdlets,
    Providers,
    Types,
    Format,
    HostWriter,
    ErrorHistory,
    Environment,
    NativeSearch,
}

impl Stage {
    #[inline]
    pub(crate) fn enter(self) {
        let bit = 1 << self as u32;
        if enabled()
            && SEEN.load(Ordering::Relaxed) & bit == 0
            && SEEN.fetch_or(bit, Ordering::Relaxed) & bit == 0
        {
            let label = match self {
                Self::Arguments => "host.arguments",
                Self::Session => "engine.session",
                Self::Parse => "syntax.parse",
                Self::Variables => "variables.table",
                Self::Functions => "functions.table",
                Self::Aliases => "aliases.lookup",
                Self::Cmdlets => "cmdlets.lookup",
                Self::Providers => "providers.lookup",
                Self::Types => "types.lookup",
                Self::Format => "format.view",
                Self::HostWriter => "host.writer",
                Self::ErrorHistory => "errors.history",
                Self::Environment => "environment.paths",
                Self::NativeSearch => "native.search",
            };
            let _ = writeln!(io::stderr().lock(), "rpsh init\t{label}");
        }
    }
}

/// Mark entry into the console host's option parser. Help and version can
/// finish here without starting a session.
pub fn host_arguments() {
    Stage::Arguments.enter();
}

pub(crate) fn module_loaded(name: &str) {
    if enabled() {
        let _ = writeln!(io::stderr().lock(), "rpsh init\tmodule.load\t{name}");
    }
}
