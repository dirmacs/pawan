# Ares dependabot vulnerability scan — 2026-04-11

Source: background research agent run from pawan main loop, scanning /opt/ares.
Tracks: pawan task #32.

## TL;DR

The 2 open Dependabot alerts on /opt/ares (`jsonwebtoken` medium, `lru` low) are
**both false positives** — orphan entries in `Cargo.lock` from earlier resolutions.
The active build graph already runs the patched versions.

A 1-command fix clears them. Two real but lower-priority issues exist that
Dependabot does not surface.

## False positives — but the 1-command fix does NOT apply (corrected 2026-04-11)

**Initial recommendation (verified to fail):** `cargo update -p jsonwebtoken -p lru`.

**Why it fails:**
- `cargo update -p jsonwebtoken` is **ambiguous** — Cargo.lock has two rows: `jsonwebtoken@9.3.1` and `jsonwebtoken@10.3.0`.
- `cargo update -p jsonwebtoken@9.3.1 --precise 10.3.0` is rejected because the lockfile still records a `^9.2` constraint from `reqsign v0.16.5 → opendal v0.55.0 → lance-io = v1.0.1 → lance v1.0.1`. Cargo cannot bridge a major version gap via `--precise`.
- **However:** `cargo tree -i jsonwebtoken@9.3.1` returns `did not match any packages`. The old version is in Cargo.lock but is **NOT in the resolved compile graph** — it's a genuine lockfile orphan carrying stale constraint metadata from an earlier resolution (when reqsign/opendal was active).

### Ground-truth reachability check

| Crate version | In `Cargo.lock` | In `cargo tree` (compile graph) | Vulnerable code path reachable? |
|---|---|---|---|
| `jsonwebtoken 9.3.1` | ✅ | ❌ (not compiled) | No — not compiled at all |
| `jsonwebtoken 10.3.0` | ✅ | ✅ | No — Claims.exp is typed usize, no nbf field, patched version |
| `lru 0.16.3` | ✅ | ✅ (only this version) | No — code uses only get/put, never IterMut |

So both alerts are genuinely benign: `jsonwebtoken 9.3.1` isn't compiled at all, and `lru 0.16.3` is already the patched version.

### Corrected fix paths (in order of safety)

1. **Dismiss the Dependabot alerts** with justification "vulnerable code path not reachable; lockfile orphan not compiled". This is the safest and lowest-blast-radius action. Previously dismissing `rustls-webpki` as `no_bandwidth` is precedent.
2. **Wide `cargo update`** in /opt/ares to regenerate Cargo.lock from scratch. This would drop the orphan `jsonwebtoken 9.3.1` row but shifts many other transitive versions — high blast radius, run full test suite after. Only worth doing if a broader lockfile refresh is already planned.
3. **Delete the orphan row manually** from Cargo.lock (edit `[[package]] name = "jsonwebtoken" version = "9.3.1"` and its dependency list). Risky because Cargo.lock is not meant to be hand-edited and any version/hash mismatch aborts the build. Not recommended.

**Recommended action:** option 1 (dismiss with justification). The cargo update approach does not work and fixing it requires either hand-editing Cargo.lock or taking a wide update hit.

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

## Recommended remediation order (for #32) — CORRECTED

1. **Now** — dismiss the 2 Dependabot alerts on /opt/ares (`jsonwebtoken`, `lru`) with justification "vulnerable code path not reachable; lockfile orphan not compiled". Link to this intel file + the `cargo tree -i` output. **Do NOT run `cargo update -p jsonwebtoken -p lru`** — it fails (see "Corrected fix paths" above).
2. **This week** — open an issue on `tursodatabase/libsql` requesting tonic 0.12 bump. Link RUSTSEC-2026-0049.
3. **Indefinite** — accept risk on `rsa 0.9.10`; revisit when RustCrypto ships a fixed release.
4. **Optional / deferred** — plan a wide `cargo update` refresh for /opt/ares as its own task, with full test-suite verification. This would drop the `jsonwebtoken 9.3.1` lockfile orphan as a side effect but has too large a blast radius for a targeted "fix the Dependabot alerts" task.

## Verification commands the agent ran

```bash
gh api /repos/dirmacs/ares/dependabot/alerts --paginate
cd /opt/ares && cargo audit --json
cd /opt/ares && cargo tree -i jsonwebtoken
cd /opt/ares && cargo tree -i lru
deagle rg "Claims" /opt/ares/src
deagle rg "IterMut|iter_mut" /opt/ares/src
```
