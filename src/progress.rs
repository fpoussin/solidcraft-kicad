use std::{
    io::{IsTerminal, Write},
    sync::mpsc,
    thread::{self, JoinHandle},
    time::Duration,
};

enum Event {
    Stage(usize, &'static str),
    Advance(&'static str),
    Label(&'static str),
    Stop,
}

/// Ephemeral progress on a terminal; redirected stderr remains suitable for scripts.
pub struct Progress {
    worker: Option<(mpsc::Sender<Event>, JoinHandle<()>)>,
}

impl Default for Progress {
    fn default() -> Self {
        Self::new()
    }
}

impl Progress {
    pub fn new() -> Self {
        Self::with_total(3)
    }

    pub fn with_total(total: usize) -> Self {
        let total = total.max(1);
        if !std::io::stderr().is_terminal() || std::env::var("TERM").is_ok_and(|s| s == "dumb") {
            return Self { worker: None };
        }
        let (sender, receiver) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("progress".into())
            .spawn(move || {
                let (mut stage, mut label, mut tick) = (0, "Reading PCB", 0);
                loop {
                    match receiver.recv_timeout(Duration::from_millis(100)) {
                        Ok(Event::Stage(next, text)) => {
                            stage = next.min(total);
                            label = text;
                        }
                        Ok(Event::Advance(text)) => {
                            stage = (stage + 1).min(total);
                            label = text;
                        }
                        Ok(Event::Label(text)) => label = text,
                        Ok(Event::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                    let mut stderr = std::io::stderr().lock();
                    let filled = stage * 18 / total;
                    let spinner = ['|', '/', '-', '\\'][tick % 4];
                    let _ = write!(
                        stderr,
                        "\r\x1b[2K[{:<18}] {stage}/{total} {spinner} {label}",
                        "=".repeat(filled)
                    );
                    let _ = stderr.flush();
                    tick += 1;
                }
                let mut stderr = std::io::stderr().lock();
                let _ = write!(stderr, "\r\x1b[2K");
                let _ = stderr.flush();
            })
            .ok();
        Self {
            worker: worker.map(|worker| (sender, worker)),
        }
    }

    pub fn stage(&self, stage: usize, label: &'static str) {
        if let Some((sender, _)) = &self.worker {
            let _ = sender.send(Event::Stage(stage, label));
        }
    }

    pub fn advance(&self, label: &'static str) {
        if let Some((sender, _)) = &self.worker {
            let _ = sender.send(Event::Advance(label));
        }
    }

    pub fn label(&self, label: &'static str) {
        if let Some((sender, _)) = &self.worker {
            let _ = sender.send(Event::Label(label));
        }
    }
}

impl Drop for Progress {
    fn drop(&mut self) {
        if let Some((sender, worker)) = self.worker.take() {
            let _ = sender.send(Event::Stop);
            let _ = worker.join();
        }
    }
}
