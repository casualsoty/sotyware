//! Tests of folder browsing: the pure key rules, and `list_folder` end to end
//! (account, credential store, S3 adapter) against the in-process listing
//! server, whose buckets hold keys exactly as given.

use std::sync::Arc;
use std::time::Duration;

use super::super::account::{AccountConfig, MemoryAccountStore};
use super::super::credentials::{CredentialError, CredentialStore, MemoryCredentialStore};
use super::super::dashboard::{AccountInput, CredentialInput};
use super::super::s3::{ClientOptions, ObjectInfo, S3Store};
use super::super::test_server::{LISTING_MODIFIED_SECS, TestServer};
use super::*;

const KEY: &str = "test-key";
const SECRET: &str = "test-secret-value";
const BUCKET: &str = "browse-bucket";

fn options() -> ClientOptions {
    ClientOptions {
        connect_timeout: Duration::from_secs(5),
        read_timeout: Duration::from_secs(5),
        max_attempts: 1,
        ..ClientOptions::default()
    }
}

fn state_with(keys: &Arc<MemoryCredentialStore>, page_size: i32) -> Arc<CloudAccounts> {
    state_with_options(keys, page_size, options())
}

fn state_with_options(
    keys: &Arc<MemoryCredentialStore>,
    page_size: i32,
    options: ClientOptions,
) -> Arc<CloudAccounts> {
    Arc::new(
        CloudAccounts::load(
            Ok(Box::new(MemoryAccountStore::default())),
            Arc::clone(keys) as Arc<dyn CredentialStore>,
            options,
        )
        .with_page_size(page_size),
    )
}

fn input(label: &str, endpoint: &str, bucket: &str) -> AccountInput {
    AccountInput {
        label: label.into(),
        provider: "s3-compatible".into(),
        region: "us-east-1".into(),
        endpoint: Some(endpoint.into()),
        bucket: bucket.into(),
    }
}

/// A state with one account (id `local`) on `server`'s `bucket`.
fn local(
    server: &TestServer,
    bucket: &str,
    page_size: i32,
) -> (Arc<CloudAccounts>, Arc<MemoryCredentialStore>) {
    let keys = Arc::new(MemoryCredentialStore::default());
    let s = state_with(&keys, page_size);
    let id = s
        .add(
            &input("Local", &server.endpoint, bucket),
            CredentialInput::new(&server.access_key_id, &server.secret),
        )
        .unwrap()
        .id;
    assert_eq!(id, "local");
    (s, keys)
}

async fn list(
    s: &Arc<CloudAccounts>,
    id: &str,
    prefix: &str,
    page: Option<&str>,
) -> Result<FolderListing, CommandError> {
    list_folder(
        Arc::clone(s),
        id.into(),
        prefix.into(),
        page.map(str::to_owned),
    )
    .await
}

/// Every page of `prefix`, following the cursors.
async fn list_all(s: &Arc<CloudAccounts>, id: &str, prefix: &str) -> (Vec<String>, usize) {
    let mut names = Vec::new();
    let mut page = None;
    let mut pages = 0;
    loop {
        let l = list(s, id, prefix, page.as_deref()).await.unwrap();
        pages += 1;
        names.extend(l.folders.iter().map(|f| f.prefix.clone()));
        names.extend(l.files.iter().map(|f| f.key.clone()));
        match l.next_page {
            Some(next) => page = Some(next),
            None => return (names, pages),
        }
    }
}

fn folder_prefixes(l: &FolderListing) -> Vec<&str> {
    l.folders.iter().map(|f| f.prefix.as_str()).collect()
}

fn file_keys(l: &FolderListing) -> Vec<&str> {
    l.files.iter().map(|f| f.key.as_str()).collect()
}

fn operations(server: &TestServer) -> Vec<String> {
    server.operations.lock().unwrap().clone()
}

/// A bucket with a little of everything.
const TREE: &[(&str, &str, u64)] = &[
    (BUCKET, "readme.txt", 12),
    (BUCKET, "notes.md", 0),
    (BUCKET, "photos/", 0), // marker of a folder that also has content
    (BUCKET, "photos/cat.jpg", 2048),
    (BUCKET, "photos/2024/beach.jpg", 3_000_000),
    (BUCKET, "photos/2024/deep/er/file.raw", 5),
    (BUCKET, "empty/", 0), // a folder that is only a marker
    (BUCKET, "docs/report 2024.pdf", 77),
];

// ---- pure rules ----

#[test]
fn breadcrumbs_and_parents_follow_every_separator() {
    let crumb = |name: &str, prefix: &str| Breadcrumb {
        name: name.into(),
        prefix: prefix.into(),
    };
    assert_eq!(breadcrumbs(""), []);
    assert_eq!(parent(""), None);
    assert_eq!(breadcrumbs("photos/"), [crumb("photos", "photos/")]);
    assert_eq!(parent("photos/"), Some(String::new()));
    assert_eq!(
        breadcrumbs("photos/2024/"),
        [crumb("photos", "photos/"), crumb("2024", "photos/2024/")]
    );
    assert_eq!(parent("photos/2024/"), Some("photos/".into()));
    // Empty names are real folders in S3; nothing is collapsed.
    assert_eq!(
        breadcrumbs("a//b/"),
        [crumb("a", "a/"), crumb("", "a//"), crumb("b", "a//b/")]
    );
    assert_eq!(parent("a//b/"), Some("a//".into()));
    assert_eq!(breadcrumbs("/"), [crumb("", "/")]);
    assert_eq!(
        breadcrumbs("Ferien 2024 ü/../"),
        [
            crumb("Ferien 2024 ü", "Ferien 2024 ü/"),
            crumb("..", "Ferien 2024 ü/../")
        ]
    );
    assert_eq!(parent("Ferien 2024 ü/../"), Some("Ferien 2024 ü/".into()));
}

#[test]
fn only_the_root_or_prefixes_ending_in_a_slash_are_folders() {
    for ok in ["", "a/", "a//", "/", "../", "with space/", "ü/日本/"] {
        assert!(validate_prefix(ok).is_ok(), "{ok:?}");
    }
    for bad in ["a", "a/b", " ", &format!("{}/", "a".repeat(1024))] {
        let e = validate_prefix(bad).unwrap_err();
        assert_eq!(e.kind, ErrorKind::InvalidInput, "{bad:?}");
    }
}

#[test]
fn entries_hide_the_marker_and_keep_nested_content_inside_its_folder() {
    let object = |key: &str| ObjectInfo {
        key: key.into(),
        size: 1,
        last_modified: None,
    };
    let page = FolderPage {
        prefixes: vec![
            "p/sub/".into(),
            "p/sub/".into(),      // duplicate
            "p//".into(),         // empty name
            "other/".into(),      // outside the folder
            "p/deeper/x/".into(), // a provider that ignored the delimiter
        ],
        objects: vec![
            object("p/"), // the folder's marker
            object("p/a b.txt"),
            object("p/sub/nested.txt"), // ignored delimiter again
            object("elsewhere.txt"),
            object("p/ü?#%&+.bin"),
        ],
        next_token: None,
    };
    let (folders, files) = entries("p/", page);
    let shown: Vec<_> = folders
        .iter()
        .map(|f| (f.name.as_str(), f.prefix.as_str()))
        .collect();
    assert_eq!(
        shown,
        [("sub", "p/sub/"), ("", "p//"), ("deeper", "p/deeper/")]
    );
    let shown: Vec<_> = files
        .iter()
        .map(|f| (f.name.as_str(), f.key.as_str()))
        .collect();
    assert_eq!(
        shown,
        [("a b.txt", "p/a b.txt"), ("ü?#%&+.bin", "p/ü?#%&+.bin")]
    );
}

#[test]
fn provider_errors_become_kinds_with_sotywares_own_words() {
    let cases = [
        (
            StorageError::InvalidCredentials("list folder: Signature validation failed".into()),
            ErrorKind::InvalidCredentials,
        ),
        (
            StorageError::AccessDenied("list folder: not entitled".into()),
            ErrorKind::AccessDenied,
        ),
        (
            StorageError::NotFound("list folder: Bucket name is invalid".into()),
            ErrorKind::BucketUnavailable,
        ),
        (
            StorageError::Network("list folder: dispatch failure 10.0.0.1".into()),
            ErrorKind::Network,
        ),
        (
            StorageError::Timeout("list folder".into()),
            ErrorKind::Timeout,
        ),
        (
            StorageError::Provider {
                status: 503,
                code: "SlowDown".into(),
                message: "list folder: please reduce your request rate".into(),
            },
            ErrorKind::Provider,
        ),
        (StorageError::Integrity("x".into()), ErrorKind::Other),
    ];
    for (error, kind) in cases {
        let e = listing_error(&error);
        assert_eq!(e.kind, kind, "{error}");
        for provider_text in [
            "Signature validation failed",
            "not entitled",
            "Bucket name is invalid",
            "10.0.0.1",
            "reduce your request rate",
        ] {
            assert!(!e.message.contains(provider_text), "{}", e.message);
        }
    }
    let e = listing_error(&StorageError::Provider {
        status: 503,
        code: "SlowDown".into(),
        message: String::new(),
    });
    assert_eq!(
        e.message,
        "the provider refused the listing: HTTP 503 (SlowDown)"
    );
    // A 404 does not claim the bucket does not exist (Backblaze answers 404
    // also for a bucket the key may not see).
    let e = listing_error(&StorageError::NotFound(String::new()));
    assert!(
        e.message.contains("or this key may not see it"),
        "{}",
        e.message
    );
}

// ---- against the listing server ----

#[tokio::test(flavor = "multi_thread")]
async fn the_root_lists_immediate_folders_and_files_only() {
    let server = TestServer::start_listing(KEY, SECRET, &[BUCKET], TREE, &[]).await;
    let (s, _keys) = local(&server, BUCKET, 1000);
    let l = list(&s, "local", "", None).await.unwrap();
    assert_eq!(
        (l.account_id.as_str(), l.bucket.as_str(), l.prefix.as_str()),
        ("local", BUCKET, "")
    );
    assert_eq!((l.parent.clone(), l.breadcrumbs.clone()), (None, vec![]));
    assert_eq!(folder_prefixes(&l), ["docs/", "empty/", "photos/"]);
    assert_eq!(file_keys(&l), ["notes.md", "readme.txt"]);
    let readme = &l.files[1];
    assert_eq!((readme.name.as_str(), readme.size), ("readme.txt", 12));
    let modified = readme.last_modified.as_deref().unwrap();
    assert_eq!(
        modified, "2025-11-11T12:00:00Z",
        "RFC 3339 from the provider"
    );
    let _ = LISTING_MODIFIED_SECS;
    assert_eq!(l.next_page, None);
    // One read-only request; never ListBuckets, nothing else.
    assert_eq!(operations(&server), ["ListObjectsV2"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn nested_folders_open_one_level_at_a_time() {
    let server = TestServer::start_listing(KEY, SECRET, &[BUCKET], TREE, &[]).await;
    let (s, _keys) = local(&server, BUCKET, 1000);
    let photos = list(&s, "local", "photos/", None).await.unwrap();
    assert_eq!(folder_prefixes(&photos), ["photos/2024/"]);
    assert_eq!(photos.folders[0].name, "2024");
    assert_eq!(
        file_keys(&photos),
        ["photos/cat.jpg"],
        "no marker, nothing nested"
    );
    assert_eq!(photos.files[0].name, "cat.jpg");
    assert_eq!(photos.parent.as_deref(), Some(""));

    let year = list(&s, "local", "photos/2024/", None).await.unwrap();
    assert_eq!(folder_prefixes(&year), ["photos/2024/deep/"]);
    assert_eq!(file_keys(&year), ["photos/2024/beach.jpg"]);
    assert_eq!(year.files[0].size, 3_000_000);
    assert_eq!(year.parent.as_deref(), Some("photos/"));
    assert_eq!(
        year.breadcrumbs
            .iter()
            .map(|c| c.prefix.as_str())
            .collect::<Vec<_>>(),
        ["photos/", "photos/2024/"]
    );

    let deepest = list(&s, "local", "photos/2024/deep/er/", None)
        .await
        .unwrap();
    assert_eq!(file_keys(&deepest), ["photos/2024/deep/er/file.raw"]);
    assert!(deepest.folders.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_buckets_marker_only_folders_and_missing_folders_are_empty() {
    let server = TestServer::start_listing(KEY, SECRET, &[BUCKET, "empty-bucket"], TREE, &[]).await;
    let (s, _keys) = local(&server, "empty-bucket", 1000);
    let l = list(&s, "local", "", None).await.unwrap();
    assert!(l.folders.is_empty() && l.files.is_empty() && l.next_page.is_none());

    let (s, _keys) = local(&server, BUCKET, 1000);
    let marker_only = list(&s, "local", "empty/", None).await.unwrap();
    assert!(marker_only.folders.is_empty() && marker_only.files.is_empty());
    // A prefix with nothing under it lists as empty, not as an error.
    let gone = list(&s, "local", "no-such-folder/", None).await.unwrap();
    assert!(gone.folders.is_empty() && gone.files.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn keys_with_spaces_unicode_and_odd_separators_are_kept_exactly() {
    let keys: &[(&str, &str, u64)] = &[
        (BUCKET, "Ferien 2024/Strand Bild (1).jpg", 1),
        (BUCKET, "Ferien 2024/ü ß 日本語 😀.txt", 2),
        (BUCKET, "Ferien 2024/a+b=c&d?e#f%20g.txt", 3),
        (BUCKET, "a//b/c.txt", 4),
        (BUCKET, "/leading.txt", 5),
        (BUCKET, "../up.txt", 6),
        (BUCKET, "back\\slash.txt", 7),
        (BUCKET, " space-only/ x", 8),
    ];
    let server = TestServer::start_listing(KEY, SECRET, &[BUCKET], keys, &[]).await;
    let (s, _keys) = local(&server, BUCKET, 1000);
    let root = list(&s, "local", "", None).await.unwrap();
    assert_eq!(
        folder_prefixes(&root),
        [" space-only/", "../", "/", "Ferien 2024/", "a/"]
    );
    assert_eq!(file_keys(&root), ["back\\slash.txt"]);
    // `..` and a leading `/` are ordinary key characters, not navigation.
    let up = list(&s, "local", "../", None).await.unwrap();
    assert_eq!(file_keys(&up), ["../up.txt"]);
    assert_eq!(up.files[0].name, "up.txt");
    let slash = list(&s, "local", "/", None).await.unwrap();
    assert_eq!(file_keys(&slash), ["/leading.txt"]);
    assert_eq!(slash.breadcrumbs[0].name, "");

    let ferien = list(&s, "local", "Ferien 2024/", None).await.unwrap();
    let names: Vec<_> = ferien.files.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Strand Bild (1).jpg",
            "a+b=c&d?e#f%20g.txt",
            "ü ß 日本語 😀.txt"
        ]
    );
    // No separator is added or dropped on the way down `a//b/`.
    let a = list(&s, "local", "a/", None).await.unwrap();
    assert_eq!(folder_prefixes(&a), ["a//"]);
    assert_eq!(a.folders[0].name, "");
    let empty_name = list(&s, "local", "a//", None).await.unwrap();
    assert_eq!(folder_prefixes(&empty_name), ["a//b/"]);
    let b = list(&s, "local", "a//b/", None).await.unwrap();
    assert_eq!(file_keys(&b), ["a//b/c.txt"]);
    assert_eq!(b.parent.as_deref(), Some("a//"));
}

#[tokio::test(flavor = "multi_thread")]
async fn pages_follow_each_other_without_duplicates_or_gaps() {
    let mut keys: Vec<(&str, String, u64)> = (0..7)
        .map(|i| (BUCKET, format!("big/file-{i}.bin"), i))
        .collect();
    keys.extend((0..3).map(|i| (BUCKET, format!("big/dir-{i}/inner.txt"), 1)));
    keys.push((BUCKET, "big/dir-0/more.txt".into(), 1));
    let keys: Vec<(&str, &str, u64)> = keys.iter().map(|(b, k, s)| (*b, k.as_str(), *s)).collect();
    let server = TestServer::start_listing(KEY, SECRET, &[BUCKET], &keys, &[]).await;
    let (s, _keys) = local(&server, BUCKET, 3);

    let first = list(&s, "local", "big/", None).await.unwrap();
    assert_eq!(first.folders.len() + first.files.len(), 3);
    assert!(first.next_page.is_some());
    let (all, pages) = list_all(&s, "local", "big/").await;
    assert_eq!(pages, 4, "10 entries in pages of 3");
    let mut expected: Vec<String> = (0..3).map(|i| format!("big/dir-{i}/")).collect();
    expected.extend((0..7).map(|i| format!("big/file-{i}.bin")));
    let mut sorted = all.clone();
    sorted.sort();
    assert_eq!(sorted, expected, "every entry exactly once");
    assert_eq!(
        s.cursor_count(),
        1,
        "only the first listing's unused cursor"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cursors_belong_to_one_account_folder_and_use() {
    let keys: Vec<String> = (0..5).map(|i| format!("f/{i}.txt")).collect();
    let objects: Vec<(&str, &str, u64)> = keys.iter().map(|k| (BUCKET, k.as_str(), 1)).collect();
    let server = TestServer::start_listing(KEY, SECRET, &[BUCKET], &objects, &[]).await;
    let (s, keys_store) = local(&server, BUCKET, 2);
    let outdated = |r: Result<FolderListing, CommandError>| {
        assert_eq!(r.unwrap_err().kind, ErrorKind::OutdatedPage);
    };

    let first = list(&s, "local", "f/", None).await.unwrap();
    let cursor = first.next_page.clone().unwrap();
    // Another folder, an unknown cursor, a made-up provider token.
    outdated(list(&s, "local", "", Some(&cursor)).await);
    outdated(list(&s, "local", "f/", Some("p999")).await);
    outdated(list(&s, "local", "f/", Some("6630")).await);
    // Used once: gone (a second "load more" with it cannot duplicate).
    let second = list(&s, "local", "f/", Some(&cursor)).await.unwrap();
    assert_eq!(file_keys(&second), ["f/2.txt", "f/3.txt"]);
    outdated(list(&s, "local", "f/", Some(&cursor)).await);

    // A second account at the same provider cannot use the first one's.
    let other = s
        .add(
            &input("Other", &server.endpoint, BUCKET),
            CredentialInput::new(KEY, SECRET),
        )
        .unwrap()
        .id;
    let next = second.next_page.clone().unwrap();
    outdated(list(&s, &other, "f/", Some(&next)).await);

    // Editing the account drops its cursors, also those not yet used.
    s.update("local", &input("Renamed", &server.endpoint, BUCKET), None)
        .unwrap();
    outdated(list(&s, "local", "f/", Some(&next)).await);
    // Refreshing starts again from the first page.
    let again = list(&s, "local", "f/", None).await.unwrap();
    assert_eq!(file_keys(&again), ["f/0.txt", "f/1.txt"]);
    drop(keys_store);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_token_the_provider_refuses_is_an_outdated_page() {
    let objects: &[(&str, &str, u64)] = &[(BUCKET, "a", 1), (BUCKET, "b", 1), (BUCKET, "c", 1)];
    let server = TestServer::start_listing(KEY, SECRET, &[BUCKET], objects, &[]).await;
    let (s, keys) = local(&server, BUCKET, 1);
    let cursor = list(&s, "local", "", None)
        .await
        .unwrap()
        .next_page
        .unwrap();
    s.replace_cursor_token(&cursor, "not hex at all");
    let e = list(&s, "local", "", Some(&cursor)).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::OutdatedPage, "{e:?}");

    // The adapter itself: a 400 from the provider, not a silent restart.
    let store = S3Store::new(
        server.resolved("direct", &[BUCKET]),
        keys.load("local").unwrap(),
        options(),
    );
    let e = store
        .list_folder(BUCKET, "", Some("zz"), 10)
        .await
        .unwrap_err();
    assert!(
        matches!(e, StorageError::Provider { status: 400, .. }),
        "{e}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn accounts_at_the_same_provider_list_with_their_own_keys() {
    let a = TestServer::start_listing(
        "key-a",
        "secret-a",
        &[BUCKET],
        &[(BUCKET, "only-in-a.txt", 1)],
        &[],
    )
    .await;
    let b = TestServer::start_listing(
        "key-b",
        "secret-b",
        &[BUCKET],
        &[(BUCKET, "only-in-b.txt", 1)],
        &[],
    )
    .await;
    let keys = Arc::new(MemoryCredentialStore::default());
    let s = state_with(&keys, 1000);
    let id_a = s
        .add(
            &input("Same", &a.endpoint, BUCKET),
            CredentialInput::new("key-a", "secret-a"),
        )
        .unwrap()
        .id;
    let id_b = s
        .add(
            &input("Same", &b.endpoint, BUCKET),
            CredentialInput::new("key-b", "secret-b"),
        )
        .unwrap()
        .id;
    assert_eq!((id_a.as_str(), id_b.as_str()), ("same", "same-2"));
    let la = list(&s, &id_a, "", None).await.unwrap();
    let lb = list(&s, &id_b, "", None).await.unwrap();
    assert_eq!(file_keys(&la), ["only-in-a.txt"]);
    assert_eq!(file_keys(&lb), ["only-in-b.txt"]);
    assert_eq!((la.account_id, lb.account_id), (id_a.clone(), id_b.clone()));
    assert_eq!(operations(&a), ["ListObjectsV2"]);
    assert_eq!(operations(&b), ["ListObjectsV2"]);

    // B's key sent to A's server is refused: each listing signs with its own.
    s.update(&id_b, &input("Same", &a.endpoint, BUCKET), None)
        .unwrap();
    let e = list(&s, &id_b, "", None).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidCredentials, "{e:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn credential_store_problems_are_errors_and_send_nothing() {
    let server = TestServer::start_listing(KEY, SECRET, &[BUCKET], TREE, &[]).await;
    for (setup, want) in [
        (0, ErrorKind::CredentialsMissing),
        (1, ErrorKind::CredentialStoreUnavailable),
        (2, ErrorKind::CredentialStoreLocked),
        (3, ErrorKind::CredentialStore),
    ] {
        let (s, keys) = local(&server, BUCKET, 1000);
        match setup {
            0 => keys.entries.lock().unwrap().clear(),
            1 => {
                *keys.fail_load.lock().unwrap() = Some(CredentialError::Unavailable {
                    backend: "test backend",
                    detail: "not running".into(),
                });
            }
            2 => {
                *keys.fail_load.lock().unwrap() = Some(CredentialError::AccessDenied {
                    backend: "test backend",
                    detail: "prompt dismissed".into(),
                });
            }
            _ => {
                *keys.fail_load.lock().unwrap() = Some(CredentialError::Backend {
                    backend: "test backend",
                    detail: "error 1312".into(),
                });
            }
        }
        let e = list(&s, "local", "", None).await.unwrap_err();
        assert_eq!(e.kind, want, "{e:?}");
    }
    assert!(
        operations(&server).is_empty(),
        "no request without credentials"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn provider_refusals_are_classified_without_their_text_or_the_secret() {
    // Wrong secret.
    let server = TestServer::start_listing(KEY, SECRET, &[BUCKET], TREE, &[]).await;
    let keys = Arc::new(MemoryCredentialStore::default());
    let s = state_with(&keys, 1000);
    s.add(
        &input("Wrong", &server.endpoint, BUCKET),
        CredentialInput::new(KEY, "wrong-secret-0123"),
    )
    .unwrap();
    let e = list(&s, "wrong", "", None).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidCredentials, "{e:?}");
    assert!(!serde_json::to_string(&e).unwrap().contains("wrong-secret"));

    // A valid key without the listing permission.
    let denied = TestServer::start_listing(KEY, SECRET, &[BUCKET], TREE, &["ListObjectsV2"]).await;
    let (s, _keys) = local(&denied, BUCKET, 1000);
    let e = list(&s, "local", "", None).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::AccessDenied, "{e:?}");
    assert!(!e.message.contains("not entitled"), "{}", e.message);

    // A bucket that is not there.
    let (s, _keys) = local(&server, "no-such-bucket", 1000);
    let e = list(&s, "local", "", None).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::BucketUnavailable, "{e:?}");

    // Nothing listening.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let keys = Arc::new(MemoryCredentialStore::default());
    let s = state_with(&keys, 1000);
    s.add(
        &input("Down", &format!("http://127.0.0.1:{port}"), BUCKET),
        CredentialInput::new(KEY, SECRET),
    )
    .unwrap();
    let e = list(&s, "down", "", None).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Network, "{e:?}");
    assert!(!e.message.contains("127.0.0.1"), "{}", e.message);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_provider_that_never_answers_is_a_timeout() {
    // Accepts connections and reads nothing back.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let hold = tokio::spawn(async move {
        let mut open = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            open.push(socket);
        }
    });
    let keys = Arc::new(MemoryCredentialStore::default());
    let s = state_with_options(
        &keys,
        1000,
        ClientOptions {
            read_timeout: Duration::from_millis(500),
            ..options()
        },
    );
    s.add(
        &input("Silent", &format!("http://{addr}"), BUCKET),
        CredentialInput::new(KEY, SECRET),
    )
    .unwrap();
    let e = list(&s, "silent", "", None).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Timeout, "{e:?}");
    hold.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn removed_changed_or_bucketless_accounts_list_nothing() {
    let server = TestServer::start_listing(KEY, SECRET, &[BUCKET], TREE, &[]).await;
    let (s, _keys) = local(&server, BUCKET, 1000);
    let first = list(&s, "local", "", None).await.unwrap();
    assert!(!first.files.is_empty());

    // A listing whose account changed while it ran is not accepted.
    let prepared = s.prepare_listing("local", "", None).unwrap();
    s.update("local", &input("Renamed", &server.endpoint, BUCKET), None)
        .unwrap();
    let e = s
        .finish_listing("local", prepared.revision, BUCKET, "", None, None)
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::OutdatedPage);

    s.remove("local").unwrap();
    let e = list(&s, "local", "", None).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::UnknownAccount);
    let e = s
        .finish_listing("local", prepared.revision, BUCKET, "", None, None)
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::UnknownAccount);
    let before = operations(&server).len();

    // An account with no bucket (stored by an older or other build).
    let store = MemoryAccountStore::default();
    let mut config = AccountConfig::new("old", "Old", "s3-compatible", "us-east-1");
    config.endpoint = Some(server.endpoint.clone());
    store.rows.lock().unwrap().push(config);
    let keys = Arc::new(MemoryCredentialStore::with(&[("old", KEY, SECRET)]));
    let s = Arc::new(CloudAccounts::load(
        Ok(Box::new(store)),
        keys as Arc<dyn CredentialStore>,
        options(),
    ));
    let e = list(&s, "old", "", None).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidConfiguration);
    // Not a folder path: refused before anything else.
    let e = list(&s, "old", "photos", None).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidInput);
    assert_eq!(operations(&server).len(), before, "no request for either");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_listing_contains_no_credential_or_provider_token() {
    let objects: &[(&str, &str, u64)] = &[(BUCKET, "a", 1), (BUCKET, "b", 1)];
    let server = TestServer::start_listing(KEY, SECRET, &[BUCKET], objects, &[]).await;
    let (s, _keys) = local(&server, BUCKET, 1);
    let l = list(&s, "local", "", None).await.unwrap();
    let json = serde_json::to_value(&l).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "accountId": "local",
            "bucket": BUCKET,
            "prefix": "",
            "parent": null,
            "breadcrumbs": [],
            "folders": [],
            "files": [{"name": "a", "key": "a", "size": 1, "lastModified": "2025-11-11T12:00:00Z"}],
            "nextPage": l.next_page.clone().unwrap(),
        })
    );
    let shown = json.to_string();
    assert!(!shown.contains(KEY) && !shown.contains(SECRET), "{shown}");
    // The provider's token for the next page is the hex of "a": not sent.
    assert!(!shown.contains("61\""), "{shown}");
}

/// The listing waits for the credential store on the blocking pool, like
/// every other account call (see dashboard.rs), so it is not tested again
/// here; this checks only that a listing sends nothing but `ListObjectsV2`
/// across pages, folders and refreshes.
#[tokio::test(flavor = "multi_thread")]
async fn browsing_sends_only_list_objects_v2() {
    let server = TestServer::start_listing(KEY, SECRET, &[BUCKET], TREE, &[]).await;
    let (s, _keys) = local(&server, BUCKET, 2);
    list_all(&s, "local", "").await;
    list_all(&s, "local", "photos/").await;
    list(&s, "local", "", None).await.unwrap();
    let ops = operations(&server);
    assert!(!ops.is_empty());
    assert!(ops.iter().all(|op| op == "ListObjectsV2"), "{ops:?}");
}
