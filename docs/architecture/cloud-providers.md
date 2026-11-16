# Cloud storage providers: evaluation (2026-11-11)

Research for the cloud storage proof of concept
([cloud-storage.md](cloud-storage.md)). Goal: the cheapest **reliable** storage
for personal files, photos, documents and project backups, today 200 GB on
Google Drive at €30/year, possibly growing to 1 TB or more, used only through
sotyware.

All facts were read on 2026-11-11 from the providers' official pages; the
prices, egress and request terms of both providers were read again on
2026-11-15 and had not changed. Since then the figures sotyware computes
with live in `src-tauri/pricing/*.json` (ADR-041,
[cloud-storage.md](cloud-storage.md#storage-usage-and-cost-estimates)); this
page keeps the research. Also read on 2026-11-15: Backblaze B2 snapshots are
ZIP files made only in the web console (no API), kept in a read-only
`b2-snapshot-…` bucket, at most 10 TB and 10 million files each, charged as
storage only; and neither provider offers an API for bucket size or billing
(Hetzner's console shows bucket statistics, updated after 15–20 minutes).
Prices change: **everything below is an estimate from published list prices, not a
quote or a guarantee.** Nothing here was checked against a real invoice; no
provider account was used.

## Sources

- Backblaze B2: [pricing](https://www.backblaze.com/cloud-storage/pricing),
  [transaction pricing](https://www.backblaze.com/cloud-storage/transaction-pricing),
  [S3-compatible API](https://www.backblaze.com/docs/cloud-storage-s3-compatible-api)
  (updated 2025-12-10), [large files](https://www.backblaze.com/docs/cloud-storage-large-files),
  [S3 UploadPart](https://www.backblaze.com/apidocs/s3-upload-part),
  [file versions](https://www.backblaze.com/docs/cloud-storage-file-versions),
  [lifecycle rules](https://www.backblaze.com/docs/cloud-storage-lifecycle-rules),
  [Object Lock](https://www.backblaze.com/docs/cloud-storage-object-lock),
  [application keys](https://www.backblaze.com/docs/cloud-storage-application-keys),
  [buckets](https://www.backblaze.com/docs/cloud-storage-buckets),
  [data regions](https://www.backblaze.com/docs/cloud-storage-data-regions).
- Hetzner: [Object Storage](https://www.hetzner.com/storage/object-storage/),
  [overview](https://docs.hetzner.com/storage/object-storage/overview/),
  [FAQ](https://docs.hetzner.com/storage/object-storage/faq/general/),
  [buckets and objects FAQ](https://docs.hetzner.com/storage/object-storage/faq/buckets-objects/),
  [S3 credentials FAQ](https://docs.hetzner.com/storage/object-storage/faq/s3-credentials/),
  [supported actions](https://docs.hetzner.com/storage/object-storage/supported-actions/),
  [libraries](https://docs.hetzner.com/storage/object-storage/getting-started/using-libraries/).
- Google One France: [plans](https://one.google.com/about/plans?hl=fr&gl=FR).
- Exchange rate: [ECB euro reference rate, USD](https://www.ecb.europa.eu/stats/policy_and_exchange_rates/euro_reference_exchange_rates/html/eurofxref-graph-usd.en.html).
- AWS SDK checksum defaults: [data integrity protection](https://docs.aws.amazon.com/sdkref/latest/guide/feature-dataintegrity.html).

## Comparison

| | Backblaze B2 | Hetzner Object Storage |
| --- | --- | --- |
| Storage | $6.95 per TB per 30 days, billed per byte-hour; first 10 GB free | €6.49/month (excl. VAT) base fee per account, includes 1 TB storage (744 TB-hours) and 1 TB egress; extra storage €0.0087 per TB-hour (≈ €6.35/TB-month) |
| Minimum charge | none | the base fee, billed hourly (€0.0104/h, capped at €6.49) for every hour at least one bucket exists, even empty |
| Egress | free up to 3× the average stored data per month, then $0.01/GB | 1 TB/month included, then €1.00/TB (the page writes "per hour, per TB", taken to be a typo) |
| Ingress | free | free |
| API requests | classes A, B, C free; class D (event notifications) 2,500/day free, then $0.004/10,000 | free |
| Minimum storage duration | none | none found |
| Minimum object size | none | 64 kB (smaller objects billed as 64 kB) |
| Old versions | buckets are always versioned; a plain delete only hides; hidden and old versions **are billed** until a lifecycle rule removes them | versioning optional (off by default) |
| Object Lock | yes (governance, compliance, legal hold, 1–3,000 days), no extra cost | yes (governance, compliance, legal hold); must be enabled when the bucket is created; deleted anyway when the contract ends |
| S3 API | SigV4 only, HTTPS only, path and virtual-hosted style; no object tagging, ACLs limited to private/public-read | SigV4 only; "basic compatibility"; no tagging, replication, website, inventory, etc.; SSE-C only; examples use virtual-hosted style only |
| Endpoint | `https://s3.<region>.backblazeb2.com` | `https://<location>.your-objectstorage.com` |
| EU regions | EU Central (Amsterdam). **One region per account, chosen at sign-up, cannot be changed.** Region id `eu-central-003` is not on an official page, but a real bucket's endpoint was `s3.eu-central-003.backblazeb2.com` and requests to it succeeded (2026-11-11); read it from the bucket's endpoint in the console | Falkenstein (fsn1), Nuremberg (nbg1), Helsinki (hel1); one data centre per bucket, no replication |
| Multipart | parts 5 MB–5 GB, at least 2 parts, part numbers up to 10,000; large files up to 10 TB | parts up to 5 GB, up to 10,000 parts; minimum part size not documented |
| Single PUT | 5 GB (documented for the native API) | 5 GB |
| Max object | 10 TB | 5 TB |
| Limits | 100 buckets per account | 100 buckets; 100 TB and 50 million objects per bucket; 750 requests/s and 10 Gbit/s per bucket; temporary upload limits in nbg1 (503 errors) |
| Keys | application keys limitable to one bucket or prefix, with expiry; master key not recommended for S3 | each key reaches every bucket of the project; restricting needs bucket policies |
| SDK checksum headers (CRC32, `aws-chunked`) | not documented | not documented |
| Billing | USD, card or invoice; no credit card needed to sign up. **VAT for French consumers not found** on official pages | EUR, prices excl. VAT; **VAT rate for French consumers not found** on official pages |

## Capabilities and their evidence

What sotyware relies on per provider is data in `cloud/provider.rs`
(`Capabilities`, ADR-029) and shown in the account form ("What … guarantees").
Each value is `yes`/`no` with its evidence (documented, or verified at the
provider on a recorded date), `assumed` (Amazon S3's documented behaviour,
not checked there) or `unknown`. Only documented or verified values switch
behaviour on (today only conditional writes depend on it, and no provider has
them established). Verified at Backblaze (2026-11-13, the opt-in upload
tests): `AbortMultipartUpload` and `ListMultipartUploads`. Nothing is verified
at Hetzner. A run against sotyware's local test servers is never evidence for
a provider.

## Cost estimate

Assumptions:

- TB = 1,000 GB; a month = 730 hours; stored data constant through the month.
- Downloads: "minimal" = none; "1×" and "3×" = that multiple of the stored
  data, per month.
- **VAT:** both providers publish prices without VAT. The table assumes 20%
  French VAT is charged on top (EU rules for digital services to consumers);
  neither provider confirms this on an official page. If Backblaze charges no
  VAT, its prices are the "excl. VAT" column.
- **Currency:** $1 = €1 / 1.1214 (ECB reference rate of 2026-11-10, read
  through a page summary). A card's foreign-currency fee (often 0–3%) is not
  included.
- Backblaze: $6.95/TB (the transaction page also says $0.005/GB; the higher
  figure is used). Hetzner extra egress at €1.00/TB.
- Not included: restic overhead (small after deduplication and compression),
  Backblaze old versions kept without a lifecycle rule, and Hetzner's 64 kB
  minimum object size (irrelevant for restic's packs of several MB, relevant for
  many tiny plain files).

| Stored | Downloads/month | B2 €/month excl. VAT | B2 €/month incl. 20% | B2 €/year incl. 20% | Hetzner €/month excl. VAT | Hetzner €/month incl. 20% | Hetzner €/year incl. 20% |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 200 GB | minimal | 1.18 | 1.41 | 16.96 | 6.49 | 7.79 | 93.46 |
| 200 GB | 1× | 1.18 | 1.41 | 16.96 | 6.49 | 7.79 | 93.46 |
| 200 GB | 3× | 1.18 | 1.41 | 16.96 | 6.49 | 7.79 | 93.46 |
| 1 TB | minimal | 6.14 | 7.36 | 88.35 | 6.49 | 7.79 | 93.46 |
| 1 TB | 1× | 6.14 | 7.36 | 88.35 | 6.49 | 7.79 | 93.46 |
| 1 TB | 3× | 6.14 | 7.36 | 88.35 | 8.49 | 10.19 | 122.26 |
| 2 TB | minimal | 12.33 | 14.80 | 177.60 | 12.84 | 15.41 | 184.91 |
| 2 TB | 1× | 12.33 | 14.80 | 177.60 | 13.84 | 16.61 | 199.31 |
| 2 TB | 3× | 12.33 | 14.80 | 177.60 | 17.84 | 21.41 | 256.91 |
| 5 TB | minimal | 30.93 | 37.11 | 445.34 | 31.89 | 38.27 | 459.27 |
| 5 TB | 1× | 30.93 | 37.11 | 445.34 | 35.89 | 43.07 | 516.87 |
| 5 TB | 3× | 30.93 | 37.11 | 445.34 | 45.89 | 55.07 | 660.87 |

Backblaze never charges for egress in these scenarios: 3× the stored data is
exactly its free allowance. The calculation is reproducible from the
assumptions above. Rounding: to the cent, per month, then × 12.

### Against Google Drive

Google One, France (official plans page, 2026-11-11, VAT treatment not stated
there): 200 GB at €2.99/month; the user pays €30/year (annual billing). The
2 TB plan shown in France is the "Google AI Plus" bundle at €9.99/month
(≈ €120/year), not a storage-only plan.

- **200 GB: Backblaze B2 is cheaper** (about €17/year with VAT, €14 without,
  vs €30), a saving of €13–16 a year. **Hetzner is about three times more
  expensive** (€93/year): its base fee pays for 1 TB whether used or not.
- **1 TB:** B2 ≈ €88/year, Hetzner ≈ €93/year. Both cost more than Google's
  2 TB bundle (€120/year for twice the space) per TB.
- **2 TB and 5 TB:** B2 ≈ €178/year at 2 TB and €445/year at 5 TB; Hetzner
  slightly more, much more with heavy downloads. **At 2 TB, Google's €120/year
  bundle is cheaper than either.** The case for B2 or Hetzner there is not
  price but control: no vendor lock-in, a standard API, restic's client-side
  encryption, Object Lock against ransomware, versioning.

## Recommendation

**First production provider: Backblaze B2 (EU Central, Amsterdam).**

- It is the only one of the two that is cheaper than Google Drive at today's
  200 GB, and it stays the cheaper of the two at every size and download level
  calculated, because it has no base fee and free egress up to 3× the stored
  data.
- Restore cost is predictable: a full restore of everything once a month is
  free.
- Keys can be restricted to one bucket (one key per sotyware account, as the
  design assumes); Object Lock is free.
- restic recommends B2's S3 API.

Conditions and risks to check before relying on it:

1. Create the account in **EU Central**: the region cannot be changed later.
2. Set the bucket's lifecycle rule to keep only the last version (or a fixed
   number of days of history). Otherwise hidden and old versions stay billed,
   and the cost of a restic `prune` never comes down.
3. Find out whether Backblaze charges French VAT (the account's billing
   settings or invoice) and the card's currency-conversion fee. Neither is on
   an official page.
4. ~~Run the opt-in integration test against a dedicated test bucket~~
   Done 2026-11-11 on WSL2 and native Windows 11
   ([status.md](../status/status.md#backblaze-b2-against-the-real-provider-2026-11-11)):
   endpoint and region, path style, single and multipart upload, SHA-256
   round trip, hide-on-delete and delete by version, and the wrong-secret
   error were verified. The SDK's own checksum headers stay off and were not
   tried.

Hetzner becomes the better choice only near 1 TB with moderate downloads, if
EU-only ownership of the company matters, or as a **second, independent
copy**. That is cheaper to judge once the data is near 1 TB. Supporting both
costs nothing in code: both are profiles of the same adapter.
