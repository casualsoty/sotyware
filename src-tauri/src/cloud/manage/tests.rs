//! Tests of the single-request file-manager commands against the in-memory
//! versioned S3 server. No real provider.

use std::collections::HashMap;
use std::time::Duration;

use super::super::account::MemoryAccountStore;
use super::super::credentials::{CredentialStore, MemoryCredentialStore};
use super::super::dashboard::{AccountInput, CredentialInput};
use super::super::s3::ClientOptions;
use super::super::test_server::{TestServer, Versioned};
use super::*;

const KEY_ID: &str = "manage-key";
const SECRET: &str = "manage-Secret-0123456789";
const BUCKET: &str = "manage-bucket";

async fn setup(versioning: bool) -> (TestServer, Versioned, Arc<CloudAccounts>) {
    let (server, data) = TestServer::start_versioned(KEY_ID, SECRET, &[(BUCKET, versioning)]).await;
    let accounts = Arc::new(CloudAccounts::load(
        Ok(Box::new(MemoryAccountStore::default())),
        Arc::new(MemoryCredentialStore::default()) as Arc<dyn CredentialStore>,
        ClientOptions {
            max_attempts: 1,
            connect_timeout: Duration::from_secs(5),
            ..ClientOptions::default()
        },
    ));
    accounts
        .add(
            &AccountInput {
                label: "Local".into(),
                provider: "s3-compatible".into(),
                region: "us-east-1".into(),
                endpoint: Some(server.endpoint.clone()),
                bucket: BUCKET.into(),
            },
            CredentialInput::new(KEY_ID, SECRET),
        )
        .unwrap();
    (server, data, accounts)
}

#[tokio::test(flavor = "multi_thread")]
async fn creating_a_folder_puts_its_marker_and_refuses_an_existing_one() {
    let (_s, data, accounts) = setup(true).await;
    data.seed(BUCKET, "has/child.txt", b"x");
    let f = create_folder(
        Arc::clone(&accounts),
        "local".into(),
        BUCKET.into(),
        "".into(),
        "größe neu".into(),
    )
    .await
    .unwrap();
    assert_eq!(f.prefix, "größe neu/");
    assert!(data.lock().buckets[BUCKET].current("größe neu/").is_some());
    let e = create_folder(
        Arc::clone(&accounts),
        "local".into(),
        BUCKET.into(),
        "".into(),
        "has".into(),
    )
    .await
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::DestinationExists);
    for bad in ["", "a/b", ".."] {
        let e = create_folder(
            Arc::clone(&accounts),
            "local".into(),
            BUCKET.into(),
            "".into(),
            bad.into(),
        )
        .await
        .unwrap_err();
        assert_eq!(e.kind, ErrorKind::InvalidInput);
    }
    let e = create_folder(
        Arc::clone(&accounts),
        "local".into(),
        BUCKET.into(),
        "no-slash".into(),
        "x".into(),
    )
    .await
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidInput);
}

#[tokio::test(flavor = "multi_thread")]
async fn properties_show_type_metadata_and_location() {
    let (_s, data, accounts) = setup(true).await;
    data.lock().put(
        BUCKET,
        "d/e/f.json",
        b"{}".to_vec(),
        Some("application/json".into()),
        HashMap::from([
            ("sotyware-sha256".into(), "ab".into()),
            ("owner".into(), "me".into()),
        ]),
    );
    let p = properties(
        Arc::clone(&accounts),
        "local".into(),
        BUCKET.into(),
        "d/e/f.json".into(),
    )
    .await
    .unwrap();
    assert_eq!(
        (p.name.as_str(), p.folder.as_str(), p.size),
        ("f.json", "d/e/", 2)
    );
    assert_eq!(p.content_type.as_deref(), Some("application/json"));
    assert_eq!(p.sha256.as_deref(), Some("ab"));
    assert!(p.version_id.is_some());
    assert!(
        p.metadata.contains(&("owner".into(), "me".into())),
        "{:?}",
        p.metadata
    );
    let e = properties(
        Arc::clone(&accounts),
        "local".into(),
        BUCKET.into(),
        "nope".into(),
    )
    .await
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::ObjectNotFound);
}

#[tokio::test(flavor = "multi_thread")]
async fn previews_are_size_limited_and_never_treat_svg_as_an_image() {
    let (_s, data, accounts) = setup(false).await;
    let png = [b"\x89PNG\r\n\x1a\n".as_slice(), &[0u8; 20]].concat();
    data.seed(BUCKET, "i.png", &png);
    data.seed(
        BUCKET,
        "big.png",
        &[
            b"\x89PNG\r\n\x1a\n".as_slice(),
            &vec![0u8; IMAGE_PREVIEW_BYTES as usize],
        ]
        .concat(),
    );
    data.seed(BUCKET, "t.txt", "héllo".as_bytes());
    data.seed(BUCKET, "x.svg", b"<svg onload=\"alert(1)\"></svg>");
    data.seed(BUCKET, "bin", &[0, 1, 2, 3]);
    let long: String = "é".repeat(TEXT_PREVIEW_BYTES as usize);
    data.seed(BUCKET, "long.txt", long.as_bytes());
    let pv = |k: &'static str| {
        preview(
            Arc::clone(&accounts),
            "local".into(),
            BUCKET.into(),
            k.into(),
        )
    };
    let p = pv("i.png").await.unwrap();
    assert_eq!(
        (p.kind, p.mime.as_deref()),
        (PreviewKind::Image, Some("image/png"))
    );
    assert_eq!(p.data.as_deref(), Some(base64(&png).as_str()));
    assert_eq!(pv("big.png").await.unwrap().kind, PreviewKind::TooLarge);
    assert_eq!(pv("t.txt").await.unwrap().text.as_deref(), Some("héllo"));
    let svg = pv("x.svg").await.unwrap();
    assert_eq!(
        svg.kind,
        PreviewKind::Text,
        "SVG is shown as text, never rendered"
    );
    assert_eq!(pv("bin").await.unwrap().kind, PreviewKind::Unsupported);
    let l = pv("long.txt").await.unwrap();
    assert!(l.truncated);
    assert!(l.text.unwrap().len() <= TEXT_PREVIEW_BYTES as usize);
}

#[test]
fn base64_and_text_detection() {
    assert_eq!(base64(b""), "");
    assert_eq!(base64(b"f"), "Zg==");
    assert_eq!(base64(b"fo"), "Zm8=");
    assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    assert_eq!(as_text("aé".as_bytes(), false).as_deref(), Some("aé"));
    assert_eq!(
        as_text(&"aé".as_bytes()[..2], true).as_deref(),
        Some("a"),
        "a character cut by the limit"
    );
    assert_eq!(as_text(&"aé".as_bytes()[..2], false), None);
    assert_eq!(as_text(b"a\0b", false), None);
    assert_eq!(image_type(b"<svg></svg>"), None);
    assert_eq!(image_type(b"GIF89a...."), Some("image/gif"));
}

#[test]
fn name_search_matches_files_and_folders_once() {
    let mut seen = HashSet::new();
    assert_eq!(
        name_hits("", "Photos/2024/photo.jpg", "photo", &mut seen),
        vec!["Photos/", "Photos/2024/photo.jpg"]
    );
    assert_eq!(
        name_hits("", "Photos/other.txt", "photo", &mut seen),
        Vec::<String>::new(),
        "Photos/ only once"
    );
    assert_eq!(
        name_hits("a/", "a/x/y", "a", &mut seen),
        Vec::<String>::new(),
        "the searched folder itself is not a hit"
    );
    assert_eq!(name_hits("a/", "b/x", "x", &mut seen), Vec::<String>::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn search_by_name_and_by_prefix_with_continuation() {
    let (_s, data, accounts) = setup(false).await;
    {
        let mut st = data.lock();
        for n in 0..10_500 {
            st.put(
                BUCKET,
                &format!("logs/{n:05}.log"),
                vec![1],
                None,
                HashMap::new(),
            );
        }
        st.put(
            BUCKET,
            "logs/report-final.PDF",
            vec![1, 2],
            None,
            HashMap::new(),
        );
        st.put(BUCKET, "other/report.pdf", vec![1], None, HashMap::new());
    }
    let searches = Arc::new(CloudSearches::default());
    let run = |prefix: &'static str, q: &'static str, mode, cursor| {
        search(
            Arc::clone(&accounts),
            Arc::clone(&searches),
            "local".into(),
            BUCKET.into(),
            prefix.into(),
            q.into(),
            mode,
            cursor,
        )
    };
    // 10,501 objects under logs/: ten pages, then a cursor.
    let first = run("logs/", "REPORT", SearchMode::Name, None)
        .await
        .unwrap();
    assert_eq!(first.scanned, 10_000);
    let next = first.next.clone().expect("more to search");
    let rest = run("logs/", "REPORT", SearchMode::Name, Some(next.clone()))
        .await
        .unwrap();
    let all: Vec<_> = first
        .hits
        .iter()
        .chain(&rest.hits)
        .map(|h| h.path.clone())
        .collect();
    assert_eq!(all, vec!["logs/report-final.PDF"]);
    assert!(rest.next.is_none());
    // A cursor is used once, and only for the same search.
    assert_eq!(
        run("logs/", "REPORT", SearchMode::Name, Some(next))
            .await
            .unwrap_err()
            .kind,
        ErrorKind::OutdatedPage
    );
    let p = run("", "other/rep", SearchMode::Prefix, None)
        .await
        .unwrap();
    assert_eq!(
        p.hits.iter().map(|h| h.path.as_str()).collect::<Vec<_>>(),
        vec!["other/report.pdf"]
    );
    assert_eq!(p.hits[0].parent, "other/");
    assert_eq!(
        run("", "", SearchMode::Name, None).await.unwrap_err().kind,
        ErrorKind::InvalidInput
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn deleted_items_list_keys_behind_a_delete_marker() {
    let (_s, data, accounts) = setup(true).await;
    data.seed(BUCKET, "f/gone.txt", b"1");
    data.seed(BUCKET, "f/gone.txt", b"22");
    data.seed(BUCKET, "f/kept.txt", b"1");
    data.seed(BUCKET, "f-other/gone.txt", b"1");
    {
        // Delete markers as a plain delete adds them.
        let store = accounts.prepare_download("local", BUCKET).unwrap();
        store
            .delete_reporting(BUCKET, "f/gone.txt", None)
            .await
            .unwrap();
        store
            .delete_reporting(BUCKET, "f-other/gone.txt", None)
            .await
            .unwrap();
        // A marker with no version behind it.
        let out = store
            .delete_reporting(BUCKET, "f/only-marker", None)
            .await
            .unwrap();
        assert!(out.marker_created);
    }
    let l = deleted(
        Arc::clone(&accounts),
        "local".into(),
        BUCKET.into(),
        "f/".into(),
    )
    .await
    .unwrap();
    assert!(!l.truncated);
    let keys: Vec<_> = l
        .entries
        .iter()
        .map(|e| (e.key.as_str(), e.restorable, e.versions, e.size))
        .collect();
    assert_eq!(
        keys,
        vec![
            ("f/gone.txt", true, 2, Some(2)),
            ("f/only-marker", false, 0, None)
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn share_links_are_signed_without_the_secret_and_expire() {
    let (_s, data, accounts) = setup(false).await;
    data.seed(BUCKET, "a b.txt", b"x");
    let l = share(
        Arc::clone(&accounts),
        "local".into(),
        BUCKET.into(),
        "a b.txt".into(),
        3_600,
    )
    .await
    .unwrap();
    assert!(l.url.contains("X-Amz-Expires=3600"), "{}", l.url);
    assert!(l.url.contains("X-Amz-Signature="));
    assert!(!l.url.contains(SECRET));
    assert!(l.expires_at.ends_with('Z'));
    // The link downloads the file without credentials.
    let body = http_get(&l.url).await;
    assert_eq!(body, b"x");
    let e = share(
        Arc::clone(&accounts),
        "local".into(),
        BUCKET.into(),
        "a b.txt".into(),
        60,
    )
    .await
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidInput);
    let e = share(
        Arc::clone(&accounts),
        "local".into(),
        BUCKET.into(),
        "nope".into(),
        3_600,
    )
    .await
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::ObjectNotFound);
}

/// A plain HTTP/1.1 GET over TCP (the test server speaks plain HTTP).
async fn http_get(url: &str) -> Vec<u8> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let rest = url.strip_prefix("http://").unwrap();
    let (host, path) = rest.split_at(rest.find('/').unwrap());
    let mut s = tokio::net::TcpStream::connect(host).await.unwrap();
    s.write_all(
        format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").as_bytes(),
    )
    .await
    .unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf);
    assert!(text.starts_with("HTTP/1.1 200"), "{text}");
    let i = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    buf[i + 4..].to_vec()
}
