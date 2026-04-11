# Ares dependabot vulnerability scan — 2026-04-11

Source: background research agent run from pawan main loop, scanning /opt/ares.
Tracks: pawan task #32.

## TL;DR

The 2 open Dependabot alerts on /opt/ares (`jsonwebtoken` medium, `lru` low) are
**both false positives** — orphan entries in `Cargo.lock` from earlier resolutions.
The active build graph already runs the patched versions.

A 1-command fix clears them. Two real but lower-priority issues exist that
Dependabot does not surface.

## False positives (immediate fix)

| Crate | Alert | Reality |
|---|---|---|
| `jsonwebtoken` | CVE-2026-25537 (`exp`/`nbf` validation bypass) | Active build is on patched **`jsonwebtoken 10.3.0`**. Old version is an orphan in `Cargo.lock`. Code-side: `Claims.exp: usize` (typed), no `nbf` field, so the bypass path is closed even on the old version. |
| `lru`     | RUSTSEC-2026-0002 (`IterMut` UAF) | Active build is on patched **`lru 0.16.3`**. Code uses only `LruCache::get/put`, never `IterMut`. |

**Fix:** in `/opt/ares` run

```bash
cargo update -p jsonwebtoken -p lru
```

This rewrites only the orphan rows in `Cargo.lock` — no source code changes,
no version bumps in `Cargo.toml`. Both Dependabot alerts auto-close on the next
push.

## Real vulnerabilities cargo audit surfaces (Dependabot does not)

### `rsa 0.9.10` — RUSTSEC-2023-0071 (Marvin timing attack, medium)

- **Path into ares:** transitive via `jsonwebtoken` (RSA-PSS verification),
  `sqlx-mysql`, and `lancedb` chains.
- **Status:** no upstream fix shipped by `RustCrypto/rsa` yet.
- **Exploitability for ares:** **low.** ares JWTs use HS256, not RSA. `sqlx-mysql`
  is in the dep graph but never instantiated (postgres only). `lancedb` runs
  locally over the filesystem; no network attacker can mount the timing oracle.
- **Action:** accept-risk. Watch RustCrypto for the eventual fix.

### `rustls-webpki 0.102.8` — RUSTSEC-2026-0049 (CRL bypass, medium)

- **Path into ares:** `libsql → tonic 0.11 → rustls 0.22 → rustls-webpki 0.102`.
- **Blocker:** `tursodatabase/libsql` is pinned on `tonic 0.11`. The fix needs
  libsql to bump to `tonic 0.12` (which carries `rustls 0.23` / `rustls-webpki 0.103`).
- **Action:** file an upstream issue against `tursodatabase/libsql` requesting
  the tonic bump. Previously dismissed in Dependabot as `no_bandwidth`.

## Recommended remediation order (for #32)

1. **Now** — `cd /opt/ares && cargo update -p jsonwebtoken -p lru` and commit
   the lockfile. Closes the 2 false-positive Dependabot alerts.
2. **This week** — open an issue on `tursodatabase/libsql` requesting tonic 0.12
   bump. Link RUSTSEC-2026-0049.
3. **Indefinite** — accept risk on `rsa 0.9.10`; revisit when RustCrypto ships
   a fixed release.

## Verification commands the agent ran

```bash
gh api /repos/dirmacs/ares/dependabot/alerts --paginate
cd /opt/ares && cargo audit --json
cd /opt/ares && cargo tree -i jsonwebtoken
cd /opt/ares && cargo tree -i lru
deagle rg "Claims" /opt/ares/src
deagle rg "IterMut|iter_mut" /opt/ares/src
```
