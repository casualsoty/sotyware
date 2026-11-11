//! An S3 server inside the test process: `s3s` speaks the protocol (including
//! SigV4 authentication and multipart uploads), `s3s-fs` keeps objects in a
//! temporary folder. Listens on 127.0.0.1 only; nothing leaves the machine.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as ConnBuilder;
use s3s::auth::SimpleAuth;
use s3s::service::S3ServiceBuilder;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use super::account::{AccountConfig, ResolvedAccount};
use super::credentials::{Credentials, Secret};

/// A running test server. Stopped and its folder removed on drop.
pub struct TestServer {
    pub endpoint: String,
    pub access_key_id: String,
    pub secret: String,
    root: PathBuf,
    task: JoinHandle<()>,
}

impl TestServer {
    /// Starts a server that accepts only `access_key_id`/`secret`, with the
    /// given (empty) buckets.
    pub async fn start(access_key_id: &str, secret: &str, buckets: &[&str]) -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "sotyware-s3-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        // s3s-fs stores a bucket as a folder.
        for bucket in buckets {
            fs::create_dir_all(root.join(bucket)).unwrap();
        }
        let fs = s3s_fs::FileSystem::new(&root).unwrap();
        let mut builder = S3ServiceBuilder::new(fs);
        builder.set_auth(SimpleAuth::from_single(access_key_id, secret));
        let service = builder.build();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = listener.accept().await else {
                    continue;
                };
                let service = service.clone();
                tokio::spawn(async move {
                    let _ = ConnBuilder::new(TokioExecutor::new())
                        .serve_connection(TokioIo::new(socket), service)
                        .await;
                });
            }
        });
        Self {
            endpoint: format!("http://{addr}"),
            access_key_id: access_key_id.into(),
            secret: secret.into(),
            root,
            task,
        }
    }

    /// The file `s3s-fs` keeps an object's content in.
    pub fn object_path(&self, bucket: &str, key: &str) -> PathBuf {
        self.root.join(bucket).join(key)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// An `s3-compatible` account pointing at this server.
    pub fn account(&self, id: &str, buckets: &[&str]) -> AccountConfig {
        let mut c = AccountConfig::new(id, id, "s3-compatible", "us-east-1");
        c.endpoint = Some(self.endpoint.clone());
        c.buckets = buckets.iter().map(|b| (*b).to_owned()).collect();
        c
    }

    pub fn resolved(&self, id: &str, buckets: &[&str]) -> ResolvedAccount {
        self.account(id, buckets).resolve().unwrap()
    }

    pub fn credentials(&self) -> Credentials {
        Credentials {
            access_key_id: self.access_key_id.clone(),
            secret_access_key: Secret::new(self.secret.clone()),
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// A fresh local folder, removed on drop.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(name: &str) -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "sotyware-cloud-test-{}-{}-{name}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    /// Writes `size` bytes of a deterministic, non-repeating pattern.
    pub fn file(&self, name: &str, size: usize, seed: u8) -> PathBuf {
        let mut x = u32::from(seed).wrapping_mul(2_654_435_761) | 1;
        let data: Vec<u8> = (0..size)
            .map(|_| {
                // xorshift: cheap, and different for every byte position.
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x >> 24) as u8
            })
            .collect();
        let path = self.path(name);
        fs::write(&path, data).unwrap();
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
