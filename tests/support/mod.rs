use caldav_mcp::{accounts::Settings, application::Application};
use serde_json::json;
use std::{
    net::TcpListener,
    path::PathBuf,
    process::{Child, Command, Stdio},
};
pub struct Radicale {
    child: Child,
    _dir: tempfile::TempDir,
    pub app: Application,
    pub base: String,
}
impl Drop for Radicale {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Radicale {
    pub async fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let python = std::env::var_os("RADICALE_PYTHON")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".venv/bin/python"));
        assert!(
            python.exists(),
            "Set RADICALE_PYTHON to a Python with Radicale 3.9.0 installed"
        );
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let users = dir.path().join("users");
        std::fs::write(
            &users,
            format!(
                "alice:{}\nbob:{}\n",
                env!("CARGO_MANIFEST_DIR"),
                env!("CARGO_MANIFEST_DIR")
            ),
        )
        .unwrap();
        let config = dir.path().join("config");
        std::fs::write(&config,format!("[server]\nhosts = 127.0.0.1:{port}\n[auth]\ntype = htpasswd\nhtpasswd_filename = {}\nhtpasswd_encryption = plain\ndelay = 0\n[rights]\ntype = owner_only\n[storage]\nfilesystem_folder = {}\n[sharing]\ntype = files\ncollection_by_map = True\npermit_create_map = True\n[logging]\nlevel = warning\n",users.display(),dir.path().join("collections").display())).unwrap();
        let log = std::fs::File::create(dir.path().join("server.log")).unwrap();
        let child = Command::new(python)
            .args(["-m", "radicale", "--config"])
            .arg(config)
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .unwrap();
        let base = format!("http://127.0.0.1:{port}/");
        let settings:Settings=serde_json::from_value(json!({"accounts":[{"id":"alice","url":base,"username":"alice","password_env":"CARGO_MANIFEST_DIR","sharing_url":format!("{base}.sharing/v1/")},{"id":"bob","url":base,"username":"bob","password_env":"CARGO_MANIFEST_DIR","sharing_url":format!("{base}.sharing/v1/")}]})).unwrap();
        let mut server = Self {
            child,
            _dir: dir,
            app: Application::new(settings).unwrap(),
            base,
        };
        for _ in 0..100 {
            if let Some(status) = server.child.try_wait().unwrap() {
                panic!(
                    "Radicale exited {status}: {}",
                    std::fs::read_to_string(server._dir.path().join("server.log")).unwrap()
                );
            }
            if reqwest::get(&server.base).await.is_ok() {
                return server;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("Radicale did not start");
    }
}
