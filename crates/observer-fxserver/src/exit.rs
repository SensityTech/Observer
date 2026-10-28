//! What an FXServer exit status means.

use observer_platform::Exit;

/// The exit in words, for the event and the console: the raw status, then what it usually means.
pub fn describe(exit: Exit) -> String {
    match exit {
        Exit::Code(code) => match code_meaning(code) {
            Some(meaning) => format!("exit code {} ({meaning})", hex_or_decimal(code)),
            None => format!("exit code {}", hex_or_decimal(code)),
        },
        Exit::Signal(signal) => match signal_name(signal) {
            Some((name, meaning)) => format!("signal {signal} ({name}, {meaning})"),
            None => format!("signal {signal}"),
        },
        Exit::Unknown => "an exit status that could not be read".to_owned(),
    }
}

/// Windows status codes are easier to look up in hex.
fn hex_or_decimal(code: u32) -> String {
    if code >= 0x4000_0000 {
        format!("0x{code:08X}")
    } else {
        code.to_string()
    }
}

fn code_meaning(code: u32) -> Option<&'static str> {
    Some(match code {
        // FXServer 35245 on Windows, after `quit` as after a fatal error.
        1 => "FXServer's usual exit on Windows, after quit or a fatal error",
        // TerminateProcess(-2) by FXServer's crash dump server, once the dump is written.
        0xFFFF_FFFE => "ended by its crash dump server after a crash",
        0xC000_0005 => "access violation",
        0xC000_001D => "illegal instruction",
        0xC000_0017 | 0xC000_009A => "out of memory",
        0xC000_0094 => "integer division by zero",
        0xC000_00FD => "stack overflow",
        0xC000_013A => "interrupted with Ctrl+C",
        0xC000_0374 => "heap corruption",
        0xC000_0409 => "stack buffer overrun or fast fail",
        0x8000_0003 => "breakpoint",
        _ => return None,
    })
}

fn signal_name(signal: i32) -> Option<(&'static str, &'static str)> {
    Some(match signal {
        1 => ("SIGHUP", "hangup"),
        2 => ("SIGINT", "interrupted"),
        4 => ("SIGILL", "illegal instruction"),
        6 => ("SIGABRT", "aborted"),
        7 => ("SIGBUS", "bus error"),
        8 => ("SIGFPE", "arithmetic error"),
        9 => ("SIGKILL", "killed"),
        // FXServer 35245 on Linux dies of it after `quit` too.
        11 => ("SIGSEGV", "segmentation fault"),
        13 => ("SIGPIPE", "broken pipe"),
        15 => ("SIGTERM", "terminated"),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_status_codes_are_named() {
        assert_eq!(
            describe(Exit::Code(0xC000_0005)),
            "exit code 0xC0000005 (access violation)"
        );
        assert_eq!(
            describe(Exit::Code(0xFFFF_FFFE)),
            "exit code 0xFFFFFFFE (ended by its crash dump server after a crash)"
        );
        assert_eq!(
            describe(Exit::Code(0xC000_00FD)),
            "exit code 0xC00000FD (stack overflow)"
        );
        assert_eq!(
            describe(Exit::Code(1)),
            "exit code 1 (FXServer's usual exit on Windows, after quit or a fatal error)"
        );
    }

    #[test]
    fn unknown_codes_are_shown_as_they_are() {
        assert_eq!(describe(Exit::Code(3)), "exit code 3");
        assert_eq!(describe(Exit::Code(0xC0DE_0001)), "exit code 0xC0DE0001");
    }

    #[test]
    fn signals_are_named() {
        assert_eq!(
            describe(Exit::Signal(11)),
            "signal 11 (SIGSEGV, segmentation fault)"
        );
        assert_eq!(describe(Exit::Signal(9)), "signal 9 (SIGKILL, killed)");
        assert_eq!(describe(Exit::Signal(64)), "signal 64");
        assert_eq!(describe(Exit::Unknown), "an exit status that could not be read");
    }
}
