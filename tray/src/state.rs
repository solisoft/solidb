//! What the icon says of the server, from what it can observe.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Stopped,
    /// Started by the icon, not answering yet.
    Starting,
    Running,
    /// Was answering, no longer does, and its process is still there.
    Silent,
    /// Asked to stop, not gone yet.
    Stopping,
    /// Its process ended without having been asked to.
    Failed(u32),
    /// Something answers on the port that the icon did not start.
    Elsewhere,
}

/// The server's process, as the icon holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Process {
    None,
    Alive,
    Exited(u32),
}

impl State {
    /// The state after one look: the process, whether the port answered,
    /// and whether the icon has asked the server to stop.
    pub fn after(self, process: Process, answering: bool, stopping: bool) -> State {
        match process {
            Process::Exited(_) if stopping => State::Stopped,
            // Ctrl-C ends the server with code 0 whoever sent it.
            Process::Exited(0) => State::Stopped,
            Process::Exited(code) => State::Failed(code),
            Process::Alive if stopping => State::Stopping,
            Process::Alive if answering => State::Running,
            Process::Alive => match self {
                State::Running | State::Silent => State::Silent,
                _ => State::Starting,
            },
            Process::None if answering => State::Elsewhere,
            Process::None => match self {
                // Kept until the next start, so that the menu goes on saying why.
                State::Failed(code) => State::Failed(code),
                _ => State::Stopped,
            },
        }
    }

    pub fn label(self, port: u16) -> String {
        match self {
            State::Stopped => "SoliDB is stopped".to_string(),
            State::Starting => "SoliDB is starting".to_string(),
            State::Running => format!("SoliDB is running on port {port}"),
            State::Silent => format!("SoliDB does not answer on port {port}"),
            State::Stopping => "SoliDB is stopping".to_string(),
            State::Failed(code) => format!("SoliDB stopped with an error ({})", reason(code)),
            State::Elsewhere => {
                format!("Port {port} is served by a program this icon did not start")
            }
        }
    }

    pub fn can_start(self) -> bool {
        matches!(self, State::Stopped | State::Failed(_))
    }

    pub fn can_stop(self) -> bool {
        matches!(self, State::Starting | State::Running | State::Silent)
    }
}

/// An exit code as a person can look it up: Windows' own codes are known
/// by their hexadecimal form, a program's by the number it returned.
pub fn reason(code: u32) -> String {
    match code {
        0xC000_0135 => "a DLL it needs is missing".to_string(),
        0xC000_0005 => "access violation".to_string(),
        0xC000_013A => "ended by Ctrl-C".to_string(),
        2 => "file not found".to_string(),
        5 => "access denied".to_string(),
        code if code >= 0x8000_0000 => format!("code 0x{code:08X}"),
        code => format!("code {code}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Process::*;
    use State::*;

    #[test]
    fn a_start_goes_through_starting_to_running() {
        let s = Stopped.after(Alive, false, false);
        assert_eq!(s, Starting);
        assert_eq!(s.after(Alive, true, false), Running);
    }

    #[test]
    fn a_server_that_stops_answering_is_not_starting_again() {
        assert_eq!(Running.after(Alive, false, false), Silent);
        assert_eq!(Silent.after(Alive, false, false), Silent);
        assert_eq!(Silent.after(Alive, true, false), Running);
    }

    #[test]
    fn an_exit_nobody_asked_for_is_a_failure_and_stays_one() {
        let s = Running.after(Exited(101), false, false);
        assert_eq!(s, Failed(101));
        assert_eq!(s.after(None, false, false), Failed(101));
        assert!(s.can_start());
    }

    #[test]
    fn an_exit_that_was_asked_for_is_a_stop_whatever_its_code() {
        assert_eq!(Running.after(Alive, true, true), Stopping);
        assert_eq!(Stopping.after(Exited(1), false, true), Stopped);
        assert_eq!(Running.after(Exited(0), false, false), Stopped);
    }

    #[test]
    fn a_failure_is_named_the_way_it_can_be_looked_up() {
        assert_eq!(reason(3221225781), "a DLL it needs is missing");
        assert_eq!(reason(0xC0000409), "code 0xC0000409");
        assert_eq!(reason(101), "code 101");
    }

    #[test]
    fn a_port_someone_else_serves_is_not_ours_to_stop() {
        let s = Stopped.after(None, true, false);
        assert_eq!(s, Elsewhere);
        assert!(!s.can_stop() && !s.can_start());
        assert_eq!(s.after(None, false, false), Stopped);
    }
}
