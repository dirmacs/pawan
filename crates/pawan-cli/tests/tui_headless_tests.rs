use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, Child, CommandBuilder, PtySize};
use tempfile::TempDir;

struct HeadlessTui {
    _workspace: TempDir,
    child: Box<dyn Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    rx: mpsc::Receiver<Vec<u8>>,
    parser: vt100::Parser,
}

impl HeadlessTui {
    fn spawn(width: u16, height: u16) -> Self {
        let workspace = TempDir::new().expect("temp workspace");
        let workspace_path = workspace.path().to_path_buf();
        let binary = PathBuf::from(
            std::env::var("CARGO_BIN_EXE_pawan").expect("cargo should expose pawan binary"),
        );

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: height,
                cols: width,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("open test pty");

        let mut cmd = CommandBuilder::new(binary.as_os_str());
        cmd.arg("--workspace");
        cmd.arg(workspace_path.as_os_str());
        cmd.arg("--model");
        cmd.arg("test-model");
        cmd.env("NO_COLOR", "1");
        cmd.env("NVIDIA_API_KEY", "pawan-headless-test-key");
        cmd.env("PAWAN_HEADLESS_TUI_TEST", "1");

        let child = pair.slave.spawn_command(cmd).expect("spawn pawan tui");
        let mut reader = pair.master.try_clone_reader().expect("clone pty reader");
        let writer = pair.master.take_writer().expect("take pty writer");
        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            let mut buf = [0; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            _workspace: workspace,
            child,
            writer,
            rx,
            parser: vt100::Parser::new(height, width, 0),
        }
    }

    fn send(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).expect("write input to pty");
        self.writer.flush().expect("flush pty input");
    }

    fn wait_for_screen(&mut self, timeout: Duration, predicate: impl Fn(&str) -> bool) -> String {
        let deadline = Instant::now() + timeout;
        let mut screen = self.parser.screen().contents();
        while Instant::now() < deadline {
            while let Ok(bytes) = self.rx.try_recv() {
                self.parser.process(&bytes);
                screen = self.parser.screen().contents();
            }
            if predicate(&screen) {
                return screen;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!("timed out waiting for screen; last screen:\n{screen}");
    }
}

fn terminal_screenshot(screen: &str) -> String {
    screen
        .lines()
        .map(|line| {
            let line = line.trim_end();
            if line.contains("│ MODEL") {
                if let Some((prefix, suffix)) = line.rsplit_once("  |  ") {
                    if suffix.len() >= "HH:MM │".len() {
                        return format!("{prefix}  |  HH:MM │");
                    }
                }
            }
            line.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

impl Drop for HeadlessTui {
    fn drop(&mut self) {
        let _ = self.writer.write_all(b"\x11");
        let _ = self.writer.flush();
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
    }
}

#[test]
fn slash_model_enter_opens_picker_in_real_pty() {
    let mut tui = HeadlessTui::spawn(100, 30);

    tui.wait_for_screen(Duration::from_secs(5), |screen| {
        screen.contains("Self-healing CLI coding agent")
    });
    tui.send(b"x");
    tui.wait_for_screen(Duration::from_secs(5), |screen| {
        screen.contains("Type your message")
    });

    tui.send(b"/model\r");
    let screen = tui.wait_for_screen(Duration::from_secs(5), |screen| {
        screen.contains("Model Picker")
    });

    assert!(screen.contains("Model Picker"), "screen:\n{screen}");
    assert!(
        screen.contains("qwen") || screen.contains("minimax"),
        "screen:\n{screen}"
    );
    insta::assert_snapshot!("slash_model_picker_real_pty", terminal_screenshot(&screen));
}
