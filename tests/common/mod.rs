use sysinfo::{ProcessExt, RefreshKind, System, SystemExt};

pub fn pgrep_chromium() {
    let sys = System::new_with_specifics(RefreshKind::new().with_processes());
    for proc in sys.get_process_by_name("chromium") {
        println!(
            "{:?} {} {:?} {}",
            proc.parent(),
            proc.pid(),
            proc.cmd(),
            proc.status()
        );
    }
    for proc in sys.get_process_by_name("Chromium") {
        println!(
            "{:?} {} {:?} {}",
            proc.parent(),
            proc.pid(),
            proc.cmd(),
            proc.status()
        );
    }
}
