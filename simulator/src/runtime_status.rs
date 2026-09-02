use nix::sys::inotify::{AddWatchFlags, InitFlags, Inotify};
use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
    thread,
};

pub struct RuntimeStatusFiles {
    qkd_ready_path: PathBuf,
    node_idle_path: PathBuf,
    node_idle_sender: Sender<()>,
    node_idle_receiver: Arc<Mutex<Receiver<()>>>,
}

impl RuntimeStatusFiles {
    pub fn new(qkd_ready_path: impl Into<PathBuf>, node_idle_path: impl Into<PathBuf>) -> Self {
        let (node_idle_sender, node_idle_receiver) = mpsc::channel();
        Self {
            qkd_ready_path: qkd_ready_path.into(),
            node_idle_path: node_idle_path.into(),
            node_idle_sender,
            node_idle_receiver: Arc::new(Mutex::new(node_idle_receiver)),
        }
    }

    pub fn initialize(&self) -> Result<(), std::io::Error> {
        self.remove_stale_node_idle()?;
        start_node_idle_watcher(&self.node_idle_path, self.node_idle_sender.clone())?;
        self.create_qkd_ready()
    }

    pub fn begin_recalibration(&self) -> Result<(), std::io::Error> {
        self.remove_stale_node_idle()?;
        self.drain_node_idle_events()?;
        remove_file_if_exists(&self.qkd_ready_path)
    }

    pub fn wait_for_node_idle(&self) -> Result<(), std::io::Error> {
        loop {
            let receiver = self.node_idle_receiver.lock().map_err(|e| {
                std::io::Error::other(format!("node idle receiver lock poisoned: {}", e))
            })?;
            receiver.recv().map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    format!("node idle watcher stopped: {}", e),
                )
            })?;
            drop(receiver);

            if self.node_idle_path.exists() {
                return Ok(());
            }
        }
    }

    pub fn create_qkd_ready(&self) -> Result<(), std::io::Error> {
        if let Some(parent) = self.qkd_ready_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.qkd_ready_path, b"ready\n")
    }

    fn remove_stale_node_idle(&self) -> Result<(), std::io::Error> {
        remove_file_if_exists(&self.node_idle_path)
    }

    fn drain_node_idle_events(&self) -> Result<(), std::io::Error> {
        let receiver = self.node_idle_receiver.lock().map_err(|e| {
            std::io::Error::other(format!("node idle receiver lock poisoned: {}", e))
        })?;
        while receiver.try_recv().is_ok() {}
        Ok(())
    }
}

fn remove_file_if_exists(path: &Path) -> Result<(), std::io::Error> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

fn inotify_to_io_error(error: nix::errno::Errno) -> std::io::Error {
    std::io::Error::from_raw_os_error(error as i32)
}

fn start_node_idle_watcher(
    node_idle_path: &Path,
    node_idle_sender: Sender<()>,
) -> Result<(), std::io::Error> {
    let parent = node_idle_path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;

    let inotify = Inotify::init(InitFlags::IN_CLOEXEC).map_err(inotify_to_io_error)?;
    inotify
        .add_watch(
            parent,
            AddWatchFlags::IN_CREATE | AddWatchFlags::IN_MOVED_TO | AddWatchFlags::IN_CLOSE_WRITE,
        )
        .map_err(inotify_to_io_error)?;

    let node_idle_path = node_idle_path.to_owned();
    let expected_name = node_idle_path
        .file_name()
        .unwrap_or_else(|| OsStr::new("node_idle"))
        .to_owned();

    thread::spawn(move || loop {
        match inotify.read_events() {
            Ok(events) => {
                for event in events {
                    if event.name.as_deref() == Some(expected_name.as_os_str())
                        && node_idle_path.exists()
                        && node_idle_sender.send(()).is_err()
                    {
                        return;
                    }
                }
            }
            Err(e) => {
                tracing::error!("Node idle watcher stopped after inotify error: {}", e);
                return;
            }
        }
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::RuntimeStatusFiles;
    use std::{fs, thread, time::Duration};

    #[test]
    fn initialize_removes_stale_idle_and_creates_ready() {
        let base =
            std::env::temp_dir().join(format!("hw_sim_status_initialize_{}", std::process::id()));
        let ready = base.join("qkd_ready");
        let idle = base.join("node_idle");
        fs::create_dir_all(&base).unwrap();
        fs::write(&idle, b"stale").unwrap();

        let status = RuntimeStatusFiles::new(&ready, &idle);
        status.initialize().unwrap();

        assert!(ready.exists());
        assert!(!idle.exists());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn wait_for_node_idle_does_not_consume_idle_file() {
        let base = std::env::temp_dir().join(format!("hw_sim_status_wait_{}", std::process::id()));
        let ready = base.join("qkd_ready");
        let idle = base.join("node_idle");
        fs::create_dir_all(&base).unwrap();

        let status = RuntimeStatusFiles::new(&ready, &idle);
        status.initialize().unwrap();
        let idle_for_thread = idle.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            fs::write(idle_for_thread, b"idle").unwrap();
        });

        status.wait_for_node_idle().unwrap();

        assert!(idle.exists());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn recalibration_follows_the_gc_handshake() {
        let base = std::env::temp_dir().join(format!(
            "hw_sim_status_recalibration_{}",
            std::process::id()
        ));
        let ready = base.join("qkd_ready");
        let idle = base.join("node_idle_alice");
        fs::create_dir_all(&base).unwrap();

        let status = RuntimeStatusFiles::new(&ready, &idle);
        status.initialize().unwrap();
        assert!(ready.exists());

        status.begin_recalibration().unwrap();
        assert!(!ready.exists());
        assert!(!idle.exists());

        let idle_for_gc = idle.clone();
        thread::spawn(move || fs::write(idle_for_gc, b"idle").unwrap());
        status.wait_for_node_idle().unwrap();

        assert!(idle.exists());
        status.create_qkd_ready().unwrap();
        assert!(ready.exists());
        assert!(idle.exists());

        let _ = fs::remove_dir_all(base);
    }
}
