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

/// What a provider supports beyond plain object storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    pub versioning: Versioning,
    /// Object Lock (governance and compliance retention, legal hold).
    pub object_lock: bool,
}

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
    capabilities: Capabilities {
        versioning: Versioning::Always,
        object_lock: true,
    },
    // One region per Backblaze account, chosen at sign-up. The region id is
    // shown with the bucket's endpoint in the web console.
    known_regions: &[
        KnownRegion {
            id: "eu-central-003",
            location: "Amsterdam, NL (assumed id; check the bucket endpoint)",
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
    capabilities: Capabilities {
        versioning: Versioning::Optional,
        object_lock: true,
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
    capabilities: Capabilities {
        versioning: Versioning::Unknown,
        object_lock: false,
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
