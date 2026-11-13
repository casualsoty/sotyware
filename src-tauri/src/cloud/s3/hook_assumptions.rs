//! What the request recorder (ADR-024) relies on in the locked AWS SDK
//! (`aws-smithy-runtime` 1.10.0, `aws-smithy-runtime-api` 1.11.3), checked
//! with a probe interceptor against the in-process S3 server. These run in
//! every test build, with or without the `request-recorder` feature, so an
//! SDK update that changes one of them fails here first:
//!
//! 1. a value stored in the `ConfigBag` at `read_before_execution` is dropped
//!    when the operation's future is dropped (its `Drop` runs), and is not
//!    dropped early on success;
//! 2. a client interceptor still runs for operations that carry their own
//!    config override (`S3Store::upload` turns stalled-stream protection
//!    off per operation);
//! 3. hooks run in the order before-execution → (before-attempt →
//!    after-attempt)+ → after-execution, with no after-execution when the
//!    future is dropped;
//! 4. the typed input is available at `read_before_execution`; the
//!    operation `Metadata` is **not** in the `ConfigBag` yet at that hook
//!    (only from the first attempt on), so the recorder names an operation
//!    at its start by the input's type;
//! 5. the attempt number (`RequestAttempts`) and the response status are
//!    readable without touching credentials, headers or bodies.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use aws_sdk_s3::config::interceptors::{
    BeforeSerializationInterceptorContextRef, BeforeTransmitInterceptorContextRef,
    FinalizerInterceptorContextRef,
};
use aws_sdk_s3::config::{ConfigBag, Intercept, RuntimeComponents};
use aws_sdk_s3::error::BoxError;
use aws_sdk_s3::operation::abort_multipart_upload::AbortMultipartUploadInput;
use aws_sdk_s3::operation::complete_multipart_upload::CompleteMultipartUploadInput;
use aws_sdk_s3::operation::create_multipart_upload::CreateMultipartUploadInput;
use aws_sdk_s3::operation::head_bucket::HeadBucketInput;
use aws_sdk_s3::operation::head_object::HeadObjectInput;
use aws_sdk_s3::operation::put_object::PutObjectInput;
use aws_sdk_s3::operation::upload_part::UploadPartInput;
use aws_smithy_runtime_api::client::orchestrator::Metadata;
use aws_smithy_runtime_api::client::retries::RequestAttempts;
use aws_smithy_types::config_bag::{Storable, StoreReplace};

use super::super::test_server::{Inject, TempDir, TestServer};
use super::{ClientOptions, S3Store, StorageError, TransferControl};

const BUCKET: &str = "hook-bucket";
const MIB: usize = 1024 * 1024;

type Log = Arc<Mutex<Vec<String>>>;

/// Stored in the `ConfigBag` per operation; pushes `dropped <op>` when the
/// last clone goes away before `read_after_execution` marked it finished.
#[derive(Clone)]
struct Guard(Arc<GuardInner>);

struct GuardInner {
    op: String,
    finished: AtomicBool,
    log: Log,
}

impl Drop for GuardInner {
    fn drop(&mut self) {
        let what = if self.finished.load(Ordering::SeqCst) {
            "released"
        } else {
            "dropped"
        };
        self.log.lock().unwrap().push(format!("{what} {}", self.op));
    }
}

impl fmt::Debug for Guard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Guard")
    }
}

impl Storable for Guard {
    type Storer = StoreReplace<Self>;
}

#[derive(Clone, Debug, Default)]
struct Probe(Log);

impl Probe {
    fn push(&self, s: String) {
        self.0.lock().unwrap().push(s);
    }
    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
    fn ops(cfg: &ConfigBag) -> String {
        cfg.load::<Metadata>()
            .map_or_else(|| "?".to_owned(), |m| m.name().to_owned())
    }
}

impl Intercept for Probe {
    fn name(&self) -> &'static str {
        "sotyware-hook-probe"
    }

    fn read_before_execution(
        &self,
        context: &BeforeSerializationInterceptorContextRef<'_>,
        cfg: &mut ConfigBag,
    ) -> Result<(), BoxError> {
        let meta = if cfg.load::<Metadata>().is_some() {
            " (metadata already present)"
        } else {
            ""
        };
        let input = context.input();
        let (op, typed) = if let Some(i) = input.downcast_ref::<HeadObjectInput>() {
            ("HeadObject", format!(" key={}", i.key().unwrap_or("-")))
        } else if let Some(i) = input.downcast_ref::<PutObjectInput>() {
            ("PutObject", format!(" key={}", i.key().unwrap_or("-")))
        } else if let Some(i) = input.downcast_ref::<UploadPartInput>() {
            (
                "UploadPart",
                format!(
                    " part={} upload={}",
                    i.part_number().unwrap_or(0),
                    i.upload_id().is_some()
                ),
            )
        } else if input.downcast_ref::<HeadBucketInput>().is_some() {
            ("HeadBucket", String::new())
        } else if input.downcast_ref::<CreateMultipartUploadInput>().is_some() {
            ("CreateMultipartUpload", String::new())
        } else if input
            .downcast_ref::<CompleteMultipartUploadInput>()
            .is_some()
        {
            ("CompleteMultipartUpload", String::new())
        } else if input.downcast_ref::<AbortMultipartUploadInput>().is_some() {
            ("AbortMultipartUpload", String::new())
        } else {
            ("?", String::new())
        };
        let op = op.to_owned();
        self.push(format!("start {op}{typed}{meta}"));
        cfg.interceptor_state()
            .store_put(Guard(Arc::new(GuardInner {
                op: op.clone(),
                finished: AtomicBool::new(false),
                log: Arc::clone(&self.0),
            })));
        Ok(())
    }

    fn read_before_attempt(
        &self,
        _: &BeforeTransmitInterceptorContextRef<'_>,
        _: &RuntimeComponents,
        cfg: &mut ConfigBag,
    ) -> Result<(), BoxError> {
        let n = cfg.load::<RequestAttempts>().map_or(0, |a| a.attempts());
        self.push(format!("attempt {} #{n}", Self::ops(cfg)));
        Ok(())
    }

    fn read_after_attempt(
        &self,
        context: &FinalizerInterceptorContextRef<'_>,
        _: &RuntimeComponents,
        cfg: &mut ConfigBag,
    ) -> Result<(), BoxError> {
        let status = context
            .response()
            .map_or_else(|| "none".to_owned(), |r| r.status().as_u16().to_string());
        self.push(format!("attempt-end {} {status}", Self::ops(cfg)));
        Ok(())
    }

    fn read_after_execution(
        &self,
        context: &FinalizerInterceptorContextRef<'_>,
        _: &RuntimeComponents,
        cfg: &mut ConfigBag,
    ) -> Result<(), BoxError> {
        if let Some(g) = cfg.load::<Guard>() {
            g.0.finished.store(true, Ordering::SeqCst);
        }
        let outcome = match context.output_or_error() {
            Some(Ok(_)) => "ok",
            Some(Err(e)) if e.is_operation_error() => "service-error",
            Some(Err(e)) if e.is_timeout_error() => "timeout",
            Some(Err(_)) => "error",
            None => "none",
        };
        self.push(format!("end {} {outcome}", Self::ops(cfg)));
        Ok(())
    }
}

fn options(max_attempts: u32) -> ClientOptions {
    ClientOptions {
        connect_timeout: Duration::from_secs(5),
        read_timeout: Duration::from_secs(30),
        max_attempts,
        multipart_threshold: 6 * MIB as u64,
        part_size: 5 * MIB as u64,
        stall_timeout: Duration::from_secs(30),
        abort_timeout: Duration::from_secs(5),
    }
}

async fn setup(max_attempts: u32) -> (TestServer, S3Store, Probe) {
    let server = TestServer::start("hook-key", "hook-Secret-0123456789", &[BUCKET]).await;
    let probe = Probe::default();
    let store = S3Store::new(
        server.resolved("hooks", &[BUCKET]),
        server.credentials(),
        options(max_attempts),
    )
    .with_interceptor(probe.clone());
    (server, store, probe)
}

/// A dropped operation's state is released by the task that owned it; give
/// it a moment before reading the log.
async fn settle(_: &Probe) {
    tokio::time::sleep(Duration::from_millis(100)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_successful_operation_runs_the_hooks_in_order_and_releases_its_guard() {
    let (_server, s, probe) = setup(3).await;
    s.check_bucket(BUCKET).await.unwrap();
    settle(&probe).await;
    assert_eq!(
        probe.take(),
        [
            "start HeadBucket",
            "attempt HeadBucket #1",
            "attempt-end HeadBucket 200",
            "end HeadBucket ok",
            "released HeadBucket",
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refused_operation_ends_with_a_service_error_after_one_attempt() {
    let (server, s, probe) = setup(3).await;
    server.inject("HeadObject", None, 1, Inject::Deny);
    let e = s.head_object(BUCKET, "k").await.unwrap_err();
    assert!(matches!(e, StorageError::AccessDenied(_)), "{e:?}");
    settle(&probe).await;
    assert_eq!(
        probe.take(),
        [
            "start HeadObject key=k",
            "attempt HeadObject #1",
            "attempt-end HeadObject 403",
            "end HeadObject service-error",
            "released HeadObject",
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_retried_operation_is_one_execution_with_numbered_attempts() {
    let (server, s, probe) = setup(3).await;
    server.inject("HeadObject", None, 1, Inject::InternalError);
    assert_eq!(s.head_object(BUCKET, "k").await.unwrap(), None);
    settle(&probe).await;
    assert_eq!(
        probe.take(),
        [
            "start HeadObject key=k",
            "attempt HeadObject #1",
            "attempt-end HeadObject 500",
            "attempt HeadObject #2",
            "attempt-end HeadObject 404",
            "end HeadObject service-error",
            "released HeadObject",
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dropping_the_future_drops_the_guard_without_after_execution() {
    let (server, s, probe) = setup(3).await;
    server.inject("HeadObject", None, 1, Inject::Hang);
    let r = tokio::time::timeout(Duration::from_millis(500), s.head_object(BUCKET, "k")).await;
    assert!(r.is_err(), "expected the timeout to drop the request");
    settle(&probe).await;
    assert_eq!(
        probe.take(),
        [
            "start HeadObject key=k",
            "attempt HeadObject #1",
            "dropped HeadObject",
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_client_interceptor_sees_uploads_with_their_operation_override() {
    let (server, s, probe) = setup(1).await;
    let dir = TempDir::new("hooks-multipart");
    let src = dir.file("src", 12 * MIB, 3);
    let control = Arc::new(TransferControl::new());
    s.upload(BUCKET, "big.bin", &src, &control, &mut |_| {})
        .await
        .unwrap();
    let small = dir.file("small", 10, 4);
    s.upload(BUCKET, "small.bin", &small, &control, &mut |_| {})
        .await
        .unwrap();
    settle(&probe).await;
    let lines = probe.take();
    let starts: Vec<&str> = lines
        .iter()
        .filter(|l| l.starts_with("start "))
        .map(String::as_str)
        .collect();
    assert_eq!(
        starts,
        [
            "start HeadObject key=big.bin",
            "start CreateMultipartUpload",
            "start UploadPart part=1 upload=true",
            "start UploadPart part=2 upload=true",
            "start UploadPart part=3 upload=true",
            "start HeadObject key=big.bin",
            "start CompleteMultipartUpload",
            "start HeadObject key=big.bin",
            "start HeadObject key=small.bin",
            "start PutObject key=small.bin",
            "start HeadObject key=small.bin",
        ],
        "{lines:#?}"
    );
    assert_eq!(lines.iter().filter(|l| l.starts_with("end ")).count(), 11);
    assert_eq!(lines.iter().filter(|l| l.starts_with("dropped")).count(), 0);
    // The server saw the same operations.
    let ops = server.ops();
    assert_eq!(ops.len(), 11, "{ops:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_part_dropped_by_cancellation_is_seen_and_the_abort_runs_its_own_hooks() {
    let (server, s, probe) = setup(1).await;
    let dir = TempDir::new("hooks-cancel");
    let src = dir.file("src", 12 * MIB, 5);
    server.inject("UploadPart", Some(2), 1, Inject::Hang);
    let control = Arc::new(TransferControl::new());
    // Cancelled as part 2 starts sending (never completely handed over, so
    // the upload drops it at once).
    let weak = Arc::downgrade(&control);
    control.during_send(2, move || {
        if let Some(c) = weak.upgrade() {
            c.cancel();
        }
    });
    let e = s
        .upload(BUCKET, "big.bin", &src, &control, &mut |_| {})
        .await
        .unwrap_err();
    assert_eq!(e.error, StorageError::Cancelled);
    settle(&probe).await;
    let lines = probe.take();
    let i_dropped = lines
        .iter()
        .position(|l| l == "dropped UploadPart")
        .unwrap_or_else(|| panic!("{lines:#?}"));
    let i_abort = lines
        .iter()
        .position(|l| l == "start AbortMultipartUpload")
        .unwrap_or_else(|| panic!("{lines:#?}"));
    assert!(
        lines.contains(&"end AbortMultipartUpload ok".to_owned()),
        "{lines:#?}"
    );
    // The part is dropped before (or while) the abort starts; both are
    // recorded, the abort completes normally.
    assert!(i_dropped < lines.len() && i_abort < lines.len());
    assert!(
        !lines
            .iter()
            .any(|l| l.contains("secret") || l.contains("Secret"))
    );
}
