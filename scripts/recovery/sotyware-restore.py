#!/usr/bin/env python3
"""Restore a sotyware backup snapshot without sotyware (ADR-031).

Needs only Python 3.8+ (standard library) and read access to the backup
bucket. It talks plain S3 (SigV4, path-style URLs) to any S3-compatible
provider: Backblaze B2, Hetzner, Amazon S3, MinIO, ...

The access key is read from the environment, never from the command line
(other users can read command lines):

    export AWS_ACCESS_KEY_ID=...        (PowerShell: $env:AWS_ACCESS_KEY_ID = "...")
    export AWS_SECRET_ACCESS_KEY=...

List the snapshots of every job in a backup:

    sotyware-restore.py --endpoint https://s3.eu-central-003.backblazeb2.com \\
        --region eu-central-003 --bucket my-backup --prefix sotyware/ --list

Restore one snapshot (or the newest of a job) into a new folder:

    sotyware-restore.py --endpoint ... --region ... --bucket my-backup \\
        --prefix sotyware/ --latest photos --out restored/
    sotyware-restore.py ... --snapshot sotyware/snapshots/photos/20261115T103000Z-ab12.json --out restored/
    sotyware-restore.py ... --export sotyware-recovery-photos-20261115T103000Z.json --out restored/

Only entries under --only (a key prefix) are restored when given. Every
file is checked against the SHA-256 in the manifest; a file that does not
match is deleted again and reported. Existing local files are never
replaced: a restored file whose name is taken gets " (n)" before its
extension. Exit status: 0 if every selected file was restored and
verified, 1 otherwise, 2 for usage errors.
"""

import argparse
import datetime
import hashlib
import hmac
import json
import os
import re
import sys
import urllib.error
import urllib.parse
import urllib.request
import xml.etree.ElementTree as ET

UNRESERVED = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~"


def quote(s, keep_slash):
    out = []
    for b in s.encode("utf-8"):
        c = chr(b)
        if c in UNRESERVED or (keep_slash and c == "/"):
            out.append(c)
        else:
            out.append("%%%02X" % b)
    return "".join(out)


class S3:
    """Signed GET requests (AWS Signature Version 4) to one bucket."""

    def __init__(self, endpoint, region, bucket, key_id, secret):
        u = urllib.parse.urlparse(endpoint.rstrip("/"))
        if u.scheme not in ("http", "https") or not u.netloc:
            raise SystemExit("--endpoint must be an http(s) URL")
        self.scheme, self.host, self.base = u.scheme, u.netloc, u.path
        self.region, self.bucket = region, bucket
        self.key_id, self.secret = key_id, secret

    def _sign(self, path, query):
        now = datetime.datetime.now(datetime.timezone.utc)
        amz_date = now.strftime("%Y%m%dT%H%M%SZ")
        day = now.strftime("%Y%m%d")
        canonical_query = "&".join(
            "%s=%s" % (quote(k, False), quote(v, False)) for k, v in sorted(query.items())
        )
        headers = {
            "host": self.host,
            "x-amz-content-sha256": "UNSIGNED-PAYLOAD",
            "x-amz-date": amz_date,
        }
        signed = ";".join(sorted(headers))
        canonical = "\n".join(
            [
                "GET",
                path,
                canonical_query,
                "".join("%s:%s\n" % (k, headers[k]) for k in sorted(headers)),
                signed,
                "UNSIGNED-PAYLOAD",
            ]
        )
        scope = "%s/%s/s3/aws4_request" % (day, self.region)
        to_sign = "\n".join(
            ["AWS4-HMAC-SHA256", amz_date, scope, hashlib.sha256(canonical.encode()).hexdigest()]
        )
        k = ("AWS4" + self.secret).encode()
        for part in (day, self.region, "s3", "aws4_request"):
            k = hmac.new(k, part.encode(), hashlib.sha256).digest()
        signature = hmac.new(k, to_sign.encode(), hashlib.sha256).hexdigest()
        headers["authorization"] = "AWS4-HMAC-SHA256 Credential=%s/%s, SignedHeaders=%s, Signature=%s" % (
            self.key_id,
            scope,
            signed,
            signature,
        )
        del headers["host"]
        return headers, canonical_query

    def get(self, key=None, query=None):
        query = query or {}
        path = quote("%s/%s" % (self.base, self.bucket), True)
        if key is not None:
            path += "/" + quote(key, True)
        headers, qs = self._sign(path, query)
        url = "%s://%s%s%s" % (self.scheme, self.host, path, ("?" + qs) if qs else "")
        return urllib.request.urlopen(urllib.request.Request(url, headers=headers), timeout=60)

    def list(self, prefix):
        """Every key under prefix (ListObjectsV2, all pages)."""
        keys, token = [], None
        while True:
            q = {"list-type": "2", "prefix": prefix}
            if token:
                q["continuation-token"] = token
            with self.get(query=q) as r:
                root = ET.fromstring(r.read())
            ns = re.match(r"\{.*\}", root.tag)
            ns = ns.group(0) if ns else ""
            for c in root.iter(ns + "Contents"):
                keys.append(c.find(ns + "Key").text)
            truncated = (root.findtext(ns + "IsTruncated") or "false") == "true"
            token = root.findtext(ns + "NextContinuationToken")
            if not truncated or not token:
                return keys


def safe_segment(s):
    s = re.sub(r'[<>:"/\\|?*\x00-\x1f]', "_", s)[:200].rstrip(". ")
    if not s:
        return "_"
    if re.match(r"^(CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(\..*)?$", s, re.I):
        s = "_" + s
    return s


def free_name(folder, name):
    candidate, n = name, 1
    stem, dot, ext = name.rpartition(".")
    while os.path.lexists(os.path.join(folder, candidate)):
        candidate = ("%s (%d).%s" % (stem, n, ext)) if stem else ("%s (%d)" % (name, n))
        n += 1
    return candidate


def check_manifest(m):
    if m.get("format") != "sotyware-backup-snapshot" or m.get("version") != 1:
        raise SystemExit("not a sotyware snapshot manifest this script understands")
    for e in m["entries"]:
        if not re.fullmatch(r"[0-9a-f]{64}", e["sha256"]) or e["blob"] != "blobs/sha256/%s/%s" % (
            e["sha256"][:2],
            e["sha256"],
        ):
            raise SystemExit("manifest entry %r is malformed" % e.get("key"))


def main(argv):
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--endpoint", required=True)
    p.add_argument("--region", required=True)
    p.add_argument("--bucket", required=True)
    p.add_argument("--prefix", default="", help="the backup's destination folder, e.g. sotyware/")
    g = p.add_mutually_exclusive_group(required=True)
    g.add_argument("--list", action="store_true", help="list snapshots and exit")
    g.add_argument("--snapshot", help="manifest key to restore")
    g.add_argument("--latest", metavar="JOB", help="restore the newest snapshot of this job id")
    g.add_argument("--export", metavar="FILE", help="a recovery file exported by sotyware")
    p.add_argument("--out", help="folder to restore into (created if needed)")
    p.add_argument("--only", default="", help="restore only keys starting with this")
    a = p.parse_args(argv)
    key_id, secret = os.environ.get("AWS_ACCESS_KEY_ID"), os.environ.get("AWS_SECRET_ACCESS_KEY")
    if not key_id or not secret:
        print("set AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY", file=sys.stderr)
        return 2
    if a.prefix and not a.prefix.endswith("/"):
        print("--prefix must end in /", file=sys.stderr)
        return 2
    s3 = S3(a.endpoint, a.region, a.bucket, key_id, secret)
    snaps = a.prefix + "snapshots/"
    if a.list:
        for k in sorted(s3.list(snaps)):
            if k.endswith(".json"):
                print(k)
        return 0
    if not a.out:
        print("--out is required to restore", file=sys.stderr)
        return 2
    if a.export:
        with open(a.export, encoding="utf-8") as f:
            manifest = json.load(f)["manifest"]
    else:
        key = a.snapshot
        if a.latest:
            found = sorted(k for k in s3.list("%s%s/" % (snaps, a.latest)) if k.endswith(".json"))
            if not found:
                print("no snapshot of job %r" % a.latest, file=sys.stderr)
                return 1
            key = found[-1]
        with s3.get(key) as r:
            manifest = json.loads(r.read())
    check_manifest(manifest)
    source_prefix = manifest["source"]["prefix"]
    os.makedirs(a.out, exist_ok=True)
    failed = restored = 0
    for e in manifest["entries"]:
        if not e["key"].startswith(a.only):
            continue
        rel = e["key"][len(source_prefix):] if e["key"].startswith(source_prefix) else e["key"]
        if not rel or rel.endswith("/"):
            continue
        parts = [safe_segment(x) for x in rel.split("/")]
        folder = os.path.join(a.out, *parts[:-1])
        os.makedirs(folder, exist_ok=True)
        path = os.path.join(folder, free_name(folder, parts[-1]))
        h, size, created = hashlib.sha256(), 0, False
        try:
            with s3.get(a.prefix + e["blob"]) as r:
                # "x": never open a file that exists; only this file is ever
                # removed again on failure.
                with open(path, "xb") as f:
                    created = True
                    while True:
                        chunk = r.read(1 << 20)
                        if not chunk:
                            break
                        h.update(chunk)
                        f.write(chunk)
                        size += len(chunk)
        except (urllib.error.URLError, OSError) as err:
            print("FAILED %s: %s" % (e["key"], err), file=sys.stderr)
            if created:
                os.remove(path)
            failed += 1
            continue
        if h.hexdigest() != e["sha256"] or size != e["size"]:
            os.remove(path)
            print("FAILED %s: content does not match the manifest" % e["key"], file=sys.stderr)
            failed += 1
            continue
        restored += 1
        print("restored %s" % e["key"])
    print("%d restored and verified, %d failed" % (restored, failed))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
