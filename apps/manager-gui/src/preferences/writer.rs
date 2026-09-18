//! One ordered writer keeps disk synchronization off the UI thread.
use super::Preferences;
use std::{path::PathBuf, sync::mpsc, thread};

enum Request {
    Save(Preferences),
    Flush(mpsc::Sender<Result<(), String>>),
}
pub struct PreferenceWriter {
    sender: Option<mpsc::Sender<Request>>,
    results: mpsc::Receiver<Result<(), String>>,
    worker: Option<thread::JoinHandle<()>>,
}
impl PreferenceWriter {
    pub fn new(path: PathBuf) -> std::io::Result<Self> {
        Self::with_save(move |prefs| prefs.save(&path))
    }
    fn with_save(
        mut save: impl FnMut(Preferences) -> Result<(), String> + Send + 'static,
    ) -> std::io::Result<Self> {
        let (sender, receiver) = mpsc::channel();
        let (done, results) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("gsm-preferences".into())
            .spawn(move || {
                let mut last_result = Ok(());
                while let Ok(request) = receiver.recv() {
                    match request {
                        Request::Save(mut prefs) => {
                            let mut barrier = None;
                            // Keep the latest queued state, but never cross an import/exit barrier.
                            while let Ok(next) = receiver.try_recv() {
                                match next {
                                    Request::Save(newer) => prefs = newer,
                                    Request::Flush(reply) => {
                                        barrier = Some(reply);
                                        break;
                                    }
                                }
                            }
                            last_result = save(prefs);
                            let _ = done.send(last_result.clone());
                            if let Some(reply) = barrier {
                                let _ = reply.send(last_result.clone());
                            }
                        }
                        Request::Flush(reply) => {
                            let _ = reply.send(last_result.clone());
                        }
                    }
                }
            })?;
        Ok(Self {
            sender: Some(sender),
            results,
            worker: Some(worker),
        })
    }
    pub fn queue(&self, prefs: Preferences) -> Result<(), String> {
        self.sender
            .as_ref()
            .unwrap()
            .send(Request::Save(prefs))
            .map_err(|_| "App settings writer stopped / アプリ設定の保存処理が停止しました".into())
    }
    pub fn queue_confirmed(
        &self,
        prefs: Preferences,
    ) -> Result<mpsc::Receiver<Result<(), String>>, String> {
        self.queue(prefs)?;
        self.barrier()
    }
    fn barrier(&self) -> Result<mpsc::Receiver<Result<(), String>>, String> {
        let (reply, receiver) = mpsc::channel();
        self.sender
            .as_ref()
            .unwrap()
            .send(Request::Flush(reply))
            .map_err(|e| e.to_string())?;
        Ok(receiver)
    }
    /// Only after the UI loop exits (and in tests); never in an input callback.
    pub fn flush(&self) -> Result<(), String> {
        self.barrier()?.recv().map_err(|e| e.to_string())?
    }
    pub fn poll(&self) -> Option<Result<(), String>> {
        self.results.try_iter().last()
    }
}
impl Drop for PreferenceWriter {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn blocked_disk_does_not_block_navigation_and_latest_state_wins() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("preferences.json");
        let saved_path = path.clone();
        let (entered, started) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let mut first = true;
        let writer = PreferenceWriter::with_save(move |prefs| {
            if first {
                first = false;
                entered.send(()).unwrap();
                wait.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            prefs.save(&saved_path)
        })
        .unwrap();
        writer.queue(Preferences::default()).unwrap();
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        for tab in 1..=4 {
            writer
                .queue(Preferences {
                    last_tab: tab,
                    remember_tab: true,
                    ..Preferences::default()
                })
                .unwrap();
        }
        // These enqueue operations completed while the disk writer was stalled.
        release.send(()).unwrap();
        writer.flush().unwrap();
        assert_eq!(Preferences::read(&path).unwrap().last_tab, 4);
        let imported = Preferences {
            language: "en".into(),
            last_tab: 2,
            remember_tab: true,
            ..Preferences::default()
        };
        let receipt = writer.queue_confirmed(imported.clone()).unwrap();
        receipt
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        drop(writer);
        assert_eq!(Preferences::read(&path).unwrap(), imported);
    }
    #[test]
    fn failures_are_reported_and_pending_save_is_flushed_on_drop() {
        let writer = PreferenceWriter::with_save(|_| Err("disk unavailable".into())).unwrap();
        writer.queue(Preferences::default()).unwrap();
        assert_eq!(writer.flush().unwrap_err(), "disk unavailable");
        assert!(writer.poll().unwrap().is_err());
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("preferences.json");
        let writer = PreferenceWriter::new(path.clone()).unwrap();
        writer.queue(Preferences::default()).unwrap();
        drop(writer);
        assert_eq!(Preferences::read(&path).unwrap(), Preferences::default());
    }
}
