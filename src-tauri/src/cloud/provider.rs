//! What sotyware knows about each S3-compatible provider, as data.
//!
//! A provider is not code: every supported provider goes through the one S3
//! adapter (`cloud::s3`), and a [`ProviderProfile`] only says how to reach it
//! (endpoint, addressing style), what it supports and its documented limits.
//! Pricing is deliberately not here (ADR-016): it changes on a different
//! schedule and must carry its own source and date.
//!
//! The figures come from the providers' official documentation as read on
//! 2026-11-11 (sources in docs/architecture/cloud-storage.md). Values the
//! documentation does not state are marked as assumptions below.

/// How the bucket name is put into the request URL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Addressing {
    /// `https://endpoint/bucket/key`. Works with bucket names that contain dots.
    Path,
    /// `https://bucket.endpoint/key`.
    VirtualHosted,
}

/// When the AWS SDK adds its own CRC32 checksum headers to requests and checks
/// them on responses.
///
/// Since early 2025 the AWS SDKs default to `WhenSupported`, which sends
/// `x-amz-checksum-*` headers and `aws-chunked` bodies that not every
/// S3-compatible service accepts. sotyware verifies integrity end to end with
/// its own SHA-256 (see `cloud::s3`), so profiles whose support is not
/// documented use `WhenRequired`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SdkChecksums {
    WhenSupported,
    WhenRequired,
}

/// Documented size limits for uploads, in bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransferLimits {
    /// Smallest part of a multipart upload (except the last one).
    pub min_part_size: u64,
    pub max_part_size: u64,
    pub max_parts: u64,
    /// Largest object a single PUT may create.
    pub max_single_put: u64,
    pub max_object_size: u64,
}

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;
const TIB: u64 = 1024 * GIB;

/// Amazon S3's own limits, used where a provider documents nothing else.
const S3_LIMITS: TransferLimits = TransferLimits {
    min_part_size: 5 * MIB,
    max_part_size: 5 * GIB,
    max_parts: 10_000,
    max_single_put: 5 * GIB,
    max_object_size: 5 * TIB,
};

/// Whether a bucket keeps old versions of an overwritten or deleted object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Versioning {
    /// Every bucket keeps versions; a plain delete only hides the object, and
    /// hidden versions are still billed as storage.
    Always,
    /// Off unless enabled on the bucket.
    Optional,
    /// Unknown (a generic S3-compatible service).
    Unknown,
}

/// Where a statement about a provider's behaviour comes from (ADR-029).
/// A run against sotyware's local test servers is never evidence about a
/// real provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Evidence {
    /// The provider's official documentation says so.
    Documented,
    /// An opt-in test against the real provider showed it (the run is
    /// recorded in docs/status/status.md).
    VerifiedAtProvider {
        /// The date of the run, `YYYY-MM-DD`.
        date: &'static str,
    },
    /// Amazon S3's documented behaviour, assumed but not checked at this
    /// provider.
    AssumedFromS3,
}

/// Whether a provider has a behaviour, and how sotyware knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Support {
    Yes(Evidence),
    No(Evidence),
    /// Nothing documented and nothing verified.
    Unknown,
}

impl Support {
    /// Whether code may rely on the behaviour: documented or verified at
    /// the provider, not merely assumed from Amazon S3.
    pub fn established(self) -> bool {
        matches!(
            self,
            Self::Yes(Evidence::Documented | Evidence::VerifiedAtProvider { .. })
        )
    }

    /// The value the dashboard shows: `yes`, `no`, `assumed` or `unknown`.
    pub fn label(self) -> &'static str {
        match self {
            Self::Yes(Evidence::AssumedFromS3) => "assumed",
            Self::Yes(_) => "yes",
            Self::No(_) => "no",
            Self::Unknown => "unknown",
        }
    }

    /// Where the statement comes from, for the dashboard.
    pub fn evidence(self) -> Option<&'static str> {
        match self {
            Self::Yes(e) | Self::No(e) => Some(match e {
                Evidence::Documented => "documented",
                Evidence::VerifiedAtProvider { .. } => "verified at the provider",
                Evidence::AssumedFromS3 => "Amazon S3 behaviour, not checked here",
            }),
            Self::Unknown => None,
        }
    }
}

/// What a provider supports beyond plain object storage, each behaviour
/// with its evidence (ADR-029). Code that depends on one of them asks here,
/// never the provider id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    pub versioning: Versioning,
    /// Object Lock (governance and compliance retention, legal hold).
    /// Whether a bucket has it switched on is read from the bucket.
    pub object_lock: Support,
    /// `DeleteObject` on a versioned bucket adds a delete marker and says so
    /// (`x-amz-delete-marker`).
    pub delete_markers: Support,
    /// `ListObjectVersions` lists versions and delete markers.
    pub version_listing: Support,
    /// `If-None-Match: *` on `PutObject` and `CompleteMultipartUpload` is
    /// enforced: the write fails with 412 when the key exists. Uploads send
    /// it only when this is established; otherwise the HEAD-then-write race
    /// remains (ADR-021).
    pub conditional_put: Support,
    /// `x-amz-copy-source-if-match` is enforced on `CopyObject`. Sent
    /// anyway (a copy is verified by `HeadObject` afterwards).
    pub copy_if_match: Support,
    /// `CopyObject` with `MetadataDirective: COPY` keeps user metadata
    /// (sotyware's `sotyware-sha256`).
    pub metadata_on_copy: Support,
    /// `AbortMultipartUpload` removes an incomplete upload and its parts.
    pub multipart_abort: Support,
    /// `ListMultipartUploads` lists incomplete multipart uploads.
    pub list_multipart_uploads: Support,
    /// Presigned `GetObject` URLs (share links) work.
    pub presigned_get: Support,
}

impl Capabilities {
    /// Each capability's name, as the dashboard lists it, with its value.
    pub fn entries(&self) -> [(&'static str, Support); 9] {
        [
            ("object-lock", self.object_lock),
            ("delete-markers", self.delete_markers),
            ("version-listing", self.version_listing),
            ("conditional-put", self.conditional_put),
            ("copy-if-match", self.copy_if_match),
            ("metadata-on-copy", self.metadata_on_copy),
            ("multipart-abort", self.multipart_abort),
            ("list-multipart-uploads", self.list_multipart_uploads),
            ("presigned-get", self.presigned_get),
        ]
    }
}

const S3: Support = Support::Yes(Evidence::AssumedFromS3);

/// How the endpoint URL is found for an account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndpointRule {
    /// `prefix + region + suffix`, for example `https://s3.` + `eu-central-003`
    /// + `.backblazeb2.com`.
    Regional {
        prefix: &'static str,
        suffix: &'static str,
    },
    /// The account must give the endpoint (a self-hosted or unlisted service).
    Custom,
}

/// A region the documentation names. Informational: a provider can add regions
/// before sotyware knows them, so any syntactically valid region is accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KnownRegion {
    pub id: &'static str,
    pub location: &'static str,
}

/// Everything sotyware needs to know about one provider.
#[derive(Debug, PartialEq, Eq)]
pub struct ProviderProfile {
    /// Stable, lowercase kebab-case. Stored in account configurations: never
    /// change a released one.
    pub id: &'static str,
    pub name: &'static str,
    pub endpoint: EndpointRule,
    pub addressing: Addressing,
    pub sdk_checksums: SdkChecksums,
    pub limits: TransferLimits,
    pub capabilities: Capabilities,
    pub known_regions: &'static [KnownRegion],
    /// The official documentation the profile was written from.
    pub documentation: &'static str,
}

pub const BACKBLAZE_B2: ProviderProfile = ProviderProfile {
    id: "backblaze-b2",
    name: "Backblaze B2",
    endpoint: EndpointRule::Regional {
        prefix: "https://s3.",
        suffix: ".backblazeb2.com",
    },
    // Both styles are documented; path style also works for bucket names with
    // dots, and it is what restic uses.
    addressing: Addressing::Path,
    // Not documented either way (checked 2026-11-11).
    sdk_checksums: SdkChecksums::WhenRequired,
    limits: TransferLimits {
        // "Large files can range from 5 MB to 10 TB"; parts 5 MB to 5 GB;
        // part numbers 1 to 10,000. The single-PUT limit is documented for the
        // native API (5 GB) and assumed to be the same for the S3 API.
        min_part_size: 5_000_000,
        max_part_size: 5_000_000_000,
        max_parts: 10_000,
        max_single_put: 5_000_000_000,
        max_object_size: 10_000_000_000_000,
    },
    // Verified at Backblaze by the opt-in upload tests on 2026-11-13
    // (status.md: "Uploads at Backblaze"): multipart aborts answered 204 and
    // left no upload; `ListMultipartUploads` was accepted with a
    // prefix-restricted key. Everything else: documented or assumed.
    capabilities: Capabilities {
        versioning: Versioning::Always,
        object_lock: Support::Yes(Evidence::Documented),
        delete_markers: S3,
        version_listing: Support::Yes(Evidence::Documented),
        // Not documented for the S3-compatible API (checked 2026-11-15);
        // never verified: uploads keep the HEAD check only.
        conditional_put: Support::Unknown,
        copy_if_match: S3,
        metadata_on_copy: S3,
        multipart_abort: Support::Yes(Evidence::VerifiedAtProvider { date: "2026-11-13" }),
        list_multipart_uploads: Support::Yes(Evidence::VerifiedAtProvider { date: "2026-11-13" }),
        presigned_get: Support::Yes(Evidence::Documented),
    },
    // One region per Backblaze account, chosen at sign-up. The region id is
    // shown with the bucket's endpoint in the web console.
    known_regions: &[
        KnownRegion {
            id: "eu-central-003",
            // Accepted by Backblaze 2026-11-11 (opt-in integration test).
            location: "Amsterdam, NL",
        },
        KnownRegion {
            id: "us-west-004",
            location: "US West",
        },
        KnownRegion {
            id: "us-east-005",
            location: "Reston, US",
        },
    ],
    documentation: "https://www.backblaze.com/docs/cloud-storage-s3-compatible-api",
};

pub const HETZNER: ProviderProfile = ProviderProfile {
    id: "hetzner",
    name: "Hetzner Object Storage",
    endpoint: EndpointRule::Regional {
        prefix: "https://",
        suffix: ".your-objectstorage.com",
    },
    // The official examples all use virtual-hosted style; path style is not
    // documented.
    addressing: Addressing::VirtualHosted,
    // Not documented either way (checked 2026-11-11).
    sdk_checksums: SdkChecksums::WhenRequired,
    limits: TransferLimits {
        // 5 GB per PUT and per part, 10,000 parts, 5 TB per object. The
        // minimum part size is not documented: Amazon's 5 MiB is assumed.
        min_part_size: 5 * MIB,
        max_part_size: 5_000_000_000,
        max_parts: 10_000,
        max_single_put: 5_000_000_000,
        max_object_size: 5_000_000_000_000,
    },
    // Nothing verified at Hetzner: no account was available for the opt-in
    // tests.
    capabilities: Capabilities {
        versioning: Versioning::Optional,
        object_lock: Support::Yes(Evidence::Documented),
        delete_markers: S3,
        version_listing: S3,
        conditional_put: Support::Unknown,
        copy_if_match: S3,
        metadata_on_copy: S3,
        multipart_abort: S3,
        list_multipart_uploads: S3,
        presigned_get: Support::Yes(Evidence::Documented),
    },
    known_regions: &[
        KnownRegion {
            id: "fsn1",
            location: "Falkenstein, DE",
        },
        KnownRegion {
            id: "nbg1",
            location: "Nuremberg, DE",
        },
        KnownRegion {
            id: "hel1",
            location: "Helsinki, FI",
        },
    ],
    documentation: "https://docs.hetzner.com/storage/object-storage/overview/",
};

/// Any other S3-compatible service, reached through an endpoint the account
/// gives (also the local test server).
pub const S3_COMPATIBLE: ProviderProfile = ProviderProfile {
    id: "s3-compatible",
    name: "S3-compatible service",
    endpoint: EndpointRule::Custom,
    addressing: Addressing::Path,
    sdk_checksums: SdkChecksums::WhenRequired,
    limits: S3_LIMITS,
    // A service sotyware knows nothing about: Amazon's behaviour is
    // assumed where a wrong assumption is caught by a later check, unknown
    // where it would remove a safety check.
    capabilities: Capabilities {
        versioning: Versioning::Unknown,
        object_lock: Support::Unknown,
        delete_markers: S3,
        version_listing: S3,
        conditional_put: Support::Unknown,
        copy_if_match: S3,
        metadata_on_copy: S3,
        multipart_abort: S3,
        list_multipart_uploads: S3,
        presigned_get: S3,
    },
    known_regions: &[],
    documentation: "https://docs.aws.amazon.com/AmazonS3/latest/API/Welcome.html",
};

/// Every provider sotyware supports.
pub const PROVIDERS: &[&ProviderProfile] = &[&BACKBLAZE_B2, &HETZNER, &S3_COMPATIBLE];

/// The profile with this id.
pub fn profile(id: &str) -> Option<&'static ProviderProfile> {
    PROVIDERS.iter().copied().find(|p| p.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_ids_are_unique_kebab_case_and_found() {
        let mut ids: Vec<_> = PROVIDERS.iter().map(|p| p.id).collect();
        for id in &ids {
            assert!(
                id.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "{id}"
            );
            assert_eq!(profile(id).map(|p| p.id), Some(*id));
        }
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), PROVIDERS.len());
        assert_eq!(profile("google-drive"), None);
    }

    #[test]
    fn limits_are_consistent() {
        for p in PROVIDERS {
            let l = p.limits;
            assert!(l.min_part_size >= MIB, "{}", p.id);
            assert!(l.min_part_size < l.max_part_size, "{}", p.id);
            assert!(l.max_single_put <= l.max_object_size, "{}", p.id);
            assert!(
                l.max_part_size.saturating_mul(l.max_parts) >= l.max_object_size,
                "{}: the largest object must fit in max_parts parts",
                p.id
            );
        }
    }

    #[test]
    fn only_documented_or_verified_behaviour_is_established() {
        assert!(Support::Yes(Evidence::Documented).established());
        assert!(Support::Yes(Evidence::VerifiedAtProvider { date: "2026-11-13" }).established());
        assert!(!Support::Yes(Evidence::AssumedFromS3).established());
        assert!(!Support::No(Evidence::Documented).established());
        assert!(!Support::Unknown.established());
        assert_eq!(Support::Yes(Evidence::AssumedFromS3).label(), "assumed");
        assert_eq!(Support::Unknown.evidence(), None);
    }

    #[test]
    fn conditional_writes_stay_off_until_verified_at_a_provider() {
        // Sending If-None-Match where it is not enforced would give a false
        // sense of safety; ADR-029 keeps it off until a provider run shows it.
        // Change this together with the recorded run, never on its own.
        for p in PROVIDERS {
            assert!(!p.capabilities.conditional_put.established(), "{}", p.id);
        }
    }

    #[test]
    fn verified_capabilities_carry_a_date() {
        for p in PROVIDERS {
            for (name, support) in p.capabilities.entries() {
                if let Support::Yes(Evidence::VerifiedAtProvider { date })
                | Support::No(Evidence::VerifiedAtProvider { date }) = support
                {
                    assert_eq!(date.len(), 10, "{} {name}", p.id);
                    assert!(date.starts_with("20"), "{} {name}", p.id);
                }
            }
        }
    }

    #[test]
    fn regional_endpoints_use_https_and_documentation_is_official() {
        for p in PROVIDERS {
            if let EndpointRule::Regional { prefix, .. } = p.endpoint {
                assert!(prefix.starts_with("https://"), "{}", p.id);
            }
            assert!(p.documentation.starts_with("https://"), "{}", p.id);
        }
        assert_eq!(S3_COMPATIBLE.endpoint, EndpointRule::Custom);
    }
}
