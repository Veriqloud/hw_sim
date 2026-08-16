use nix::fcntl::OFlag;
use serde_json::json;
use sim_lib::BATCH_SIZE;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write},
    os::unix::{
        fs::{MetadataExt, OpenOptionsExt},
        net::UnixStream,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

const IO_TIMEOUT: Duration = Duration::from_secs(5);
const RETRY_DELAY: Duration = Duration::from_millis(10);
const GENERATION_START_OFFSET: u64 = 0x1000 + 24;
const PAUSE_DURATION: Duration = Duration::from_millis(50);

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("hw_sim_recalibration_{}", Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct FifoPeers {
    angles_reader: File,
    gc_writer: File,
}

impl FifoPeers {
    fn open(angles_path: &Path, gc_path: &Path) -> Self {
        Self {
            angles_reader: open_fifo_reader(angles_path),
            gc_writer: open_fifo_writer(gc_path),
        }
    }

    fn exchange_batch(&mut self) {
        write_with_timeout(&mut self.gc_writer, &gc_batch()).unwrap();
        read_with_timeout(&mut self.angles_reader, BATCH_SIZE / 2).unwrap();
    }
}

#[test]
fn recalibration_replaces_fifos_and_resumes_source_session() {
    let test_dir = TestDir::new();
    let command_path = test_dir.path("fpga_alice");
    let angles_path = test_dir.path("angles.fifo");
    let gc_path = test_dir.path("gc.fifo");
    let ready_path = test_dir.path("qkd_ready");
    let idle_path = test_dir.path("node_idle");
    let control_socket_path = test_dir.path("control.socket");
    let config_path = test_dir.path("config.json");

    let config = json!({
        "backend_config": {
            "angles": [0, 32, 96, 64],
            "seed": 42,
            "eta": 0.1,
            "qberr": 0.05,
            "pulse_distance": 1e-8
        },
        "ipc_config": {
            "command_path": command_path.to_str().unwrap(),
            "angle_file_path": angles_path.to_str().unwrap(),
            "gc_read_file_path": gc_path.to_str().unwrap(),
            "hw_params_file_path": test_dir.path("hw_params.txt").to_str().unwrap(),
            "control_socket_path": control_socket_path.to_str().unwrap(),
            "qkd_ready_path": ready_path.to_str().unwrap(),
            "node_idle_path": idle_path.to_str().unwrap()
        },
        "log_level": "Error"
    });
    fs::write(&config_path, config.to_string()).unwrap();

    let _simulator = ChildGuard(
        Command::new(env!("CARGO_BIN_EXE_simulator"))
            .args([
                "--config-path",
                config_path.to_str().unwrap(),
                "--logs-location",
                test_dir.0.to_str().unwrap(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );

    wait_for_path_state(&ready_path, true);
    let old_angles_fifo = open_path_reference(&angles_path);
    let old_gc_fifo = open_path_reference(&gc_path);
    let mut peers = FifoPeers::open(&angles_path, &gc_path);

    write_u32(&command_path, GENERATION_START_OFFSET, 1);
    peers.exchange_batch();

    request_pause(&control_socket_path, PAUSE_DURATION);
    // The runner may already be reading the next batch when the pause arrives.
    let _ = write_with_timeout(&mut peers.gc_writer, &gc_batch());
    wait_for_path_state(&ready_path, false);
    drop(peers);

    thread::sleep(Duration::from_millis(30));
    assert!(!ready_path.exists());
    assert_eq!(
        old_angles_fifo.metadata().unwrap().ino(),
        fs::metadata(&angles_path).unwrap().ino()
    );
    assert_eq!(
        old_gc_fifo.metadata().unwrap().ino(),
        fs::metadata(&gc_path).unwrap().ino()
    );

    let pause_started = Instant::now();
    fs::write(&idle_path, b"idle\n").unwrap();
    wait_for_path_state(&ready_path, true);

    assert!(pause_started.elapsed() >= PAUSE_DURATION);
    assert!(idle_path.exists());
    assert_ne!(
        old_angles_fifo.metadata().unwrap().ino(),
        fs::metadata(&angles_path).unwrap().ino()
    );
    assert_ne!(
        old_gc_fifo.metadata().unwrap().ino(),
        fs::metadata(&gc_path).unwrap().ino()
    );

    FifoPeers::open(&angles_path, &gc_path).exchange_batch();
}

fn gc_batch() -> Vec<u8> {
    let mut batch = Vec::with_capacity(BATCH_SIZE * 16);
    for gc in 0..BATCH_SIZE as u64 {
        batch.extend_from_slice(&gc.to_le_bytes());
        batch.extend_from_slice(&[0; 8]);
    }
    batch
}

fn open_fifo_reader(path: &Path) -> File {
    retry_io("opening angles FIFO reader", || {
        OpenOptions::new()
            .read(true)
            .custom_flags(OFlag::O_NONBLOCK.bits())
            .open(path)
    })
}

fn open_fifo_writer(path: &Path) -> File {
    retry_io("opening GC FIFO writer", || {
        OpenOptions::new()
            .write(true)
            .custom_flags(OFlag::O_NONBLOCK.bits())
            .open(path)
    })
}

fn open_path_reference(path: &Path) -> File {
    OpenOptions::new()
        .read(true)
        .custom_flags(OFlag::O_PATH.bits())
        .open(path)
        .unwrap()
}

fn retry_io<T>(operation: &str, mut attempt: impl FnMut() -> io::Result<T>) -> T {
    let deadline = Instant::now() + IO_TIMEOUT;
    loop {
        match attempt() {
            Ok(value) => return value,
            Err(_) if Instant::now() < deadline => thread::sleep(RETRY_DELAY),
            Err(error) => panic!("{operation} failed: {error}"),
        }
    }
}

fn write_with_timeout(file: &mut File, mut data: &[u8]) -> io::Result<()> {
    let deadline = Instant::now() + IO_TIMEOUT;
    while !data.is_empty() {
        match file.write(data) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(written) => data = &data[written..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                thread::sleep(RETRY_DELAY);
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn read_with_timeout(file: &mut File, size: usize) -> io::Result<()> {
    let deadline = Instant::now() + IO_TIMEOUT;
    let mut data = vec![0; size];
    let mut read = 0;
    while read < data.len() {
        match file.read(&mut data[read..]) {
            Ok(0) if Instant::now() < deadline => thread::sleep(RETRY_DELAY),
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(count) => read += count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                thread::sleep(RETRY_DELAY);
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn request_pause(socket_path: &Path, duration: Duration) {
    let mut stream = retry_io("connecting to runtime control socket", || {
        UnixStream::connect(socket_path)
    });
    stream.set_read_timeout(Some(IO_TIMEOUT)).unwrap();
    writeln!(
        stream,
        "{{\"command\":\"pause\",\"duration_ms\":{}}}",
        duration.as_millis()
    )
    .unwrap();

    let mut response = String::new();
    BufReader::new(stream).read_line(&mut response).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&response).unwrap()["status"],
        "ok"
    );
}

fn write_u32(path: &Path, offset: u64, value: u32) {
    let mut file = OpenOptions::new().write(true).open(path).unwrap();
    file.seek(SeekFrom::Start(offset)).unwrap();
    file.write_all(&value.to_le_bytes()).unwrap();
    file.flush().unwrap();
}

fn wait_for_path_state(path: &Path, should_exist: bool) {
    let deadline = Instant::now() + IO_TIMEOUT;
    while Instant::now() < deadline {
        if path.exists() == should_exist {
            return;
        }
        thread::sleep(RETRY_DELAY);
    }
    panic!(
        "{} did not become {} in time",
        path.display(),
        if should_exist { "present" } else { "absent" }
    );
}
