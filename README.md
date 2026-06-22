# Afilmory-lite

English · [简体中文](README.zh-CN.md)

[![CI](https://github.com/kylinholmes/afilmory-lite/actions/workflows/release.yml/badge.svg)](https://github.com/kylinholmes/afilmory-lite/actions/workflows/release.yml)
[![Release](https://img.shields.io/github/v/release/kylinholmes/afilmory-lite?sort=semver&logo=github)](https://github.com/kylinholmes/afilmory-lite/releases)
[![Container](https://img.shields.io/badge/ghcr.io-afilmory--lite-2496ED?logo=docker&logoColor=white)](https://github.com/kylinholmes/afilmory-lite/pkgs/container/afilmory-lite)
[![License](https://img.shields.io/github/license/kylinholmes/afilmory-lite)](LICENSE)
[![Stars](https://img.shields.io/github/stars/kylinholmes/afilmory-lite?logo=github)](https://github.com/kylinholmes/afilmory-lite/stargazers)
[![Last commit](https://img.shields.io/github/last-commit/kylinholmes/afilmory-lite)](https://github.com/kylinholmes/afilmory-lite/commits/main)
[![Visits](https://hits.sh/github.com/kylinholmes/afilmory-lite.svg?label=visits)](https://hits.sh/github.com/kylinholmes/afilmory-lite/)

afilmory-lite is a single-binary daemon for the [Afilmory](https://github.com/Afilmory/Afilmory) photo gallery, written in Rust. It serves the prebuilt Afilmory frontend and reimplements its build pipeline in-process: pulling photos from object storage and processing them into `manifest.json` and thumbnails. Data updates are triggered by scheduled polling, webhooks, S3 events, or a manual endpoint, and run incrementally — with no frontend rebuild.

## Background

Afilmory is a self-hosted photo gallery. Its default update flow re-runs a full Node build on every photo change (pull from S3, generate the manifest and thumbnails, repackage the frontend) and redeploys — which is costly when you have many photos or update frequently.

afilmory-lite decouples "frontend build" from "data update":

- The static frontend shell is built once; data is injected at runtime rather than embedded.
- A resident Rust service pulls photos from storage, processes them, generates the manifest and thumbnails, and injects them into the frontend.
- Data updates are triggered by webhook, polling, S3 events, or a manual endpoint — never touching the frontend build.

The deliverable is a single binary that depends on `exiftool` at runtime (plus `libheif` for HEIC); a prebuilt Docker image is also provided.

## Architecture

```
afilmory-lite (single process)
├─ server     serve frontend dist + runtime-inject __MANIFEST__/__SITE_CONFIG__ + SPA route fallback + cache headers + /thumbnails (+ local originals)
├─ scheduler  polling / webhook / S3 event / manual → coalescing serial build coordinator
├─ builder    list storage → incremental filter → concurrent processing → write manifest.json + thumbnails/
├─ storage    S3 / S3-compatible (hand-written SigV4), local directory
├─ pipeline   decode (optional HEIC) → thumbnail → thumbHash → tone → HDR/Live/Motion Photo → assemble
└─ exif       exiftool subprocess (EXIF fidelity)
```

Originals do not pass through this service: in S3 mode `originalUrl` points directly at the bucket or CDN; in local mode the service hosts them. The generated manifest is structurally and semantically consistent with upstream (field structure, thumbnail appearance, EXIF display values), without aiming for byte-for-byte equality.

## Quick start (Docker Compose)

The image bundles the program, frontend, `exiftool`, and `libheif`.

```bash
git clone https://github.com/kylinholmes/afilmory-lite
cd afilmory-lite

cp docker/afilmory.example.toml afilmory.toml   # edit [storage] / [site] / [triggers] as needed; you can also configure via the web page (closing the config entry after updating is recommended)
mkdir -p data photos                            # data = persistent output; photos = local photos (omit when using S3)

docker compose up -d
docker compose logs -f
```

Once started, open `http://<host>:8080/`. Images are published at `ghcr.io/kylinholmes/afilmory-lite`: `:main` is the rolling build of the main branch; pushing a `v*` tag additionally provides `:latest` and `:x.y.z`.

## Configuration

Configuration is TOML; see [`docker/afilmory.example.toml`](docker/afilmory.example.toml) for a complete example.

| Section | Field | Description |
|---|---|---|
| `[server]` | `listen` | Listen address, default `0.0.0.0:8080` |
| | `workdir` | Holds `manifest.json` and `thumbnails/` (must be persisted) |
| | `dist_dir` | Frontend static-shell directory (built into Docker at `/app/web/dist`) |
| | `admin_token` | When set, enables the `/admin` config page and config read/write API (Bearer auth), with runtime hot reload |
| `[site]` | `name` / `title` / `description` / `accentColor` / … | Injected into `window.__SITE_CONFIG__` (site info) |
| `[storage.local]` | `base_path` / `base_url` | Local directory; when `base_url` is a root path (e.g. `/photos`), the service hosts the originals |
| `[storage.s3]` | `bucket` / `region` / `endpoint` / `access_key_id` / `secret_access_key` / `prefix` / `custom_domain` … | S3 and compatible services (AWS / MinIO / Cloudflare R2 / Wasabi) |
| `[processing]` | `concurrency` / `thumbnail_width`(600) / `thumbnail_quality`(100) / `enable_live_photo` | Processing parameters |
| `[exif]` | `exiftool_path` | Path to the exiftool executable (default `exiftool`) |
| `[triggers]` | `poll_interval_secs` | Enables scheduled polling when greater than 0 |
| | `webhook_token` | When set, enables `/api/hooks/build` and `/api/admin/build` (Bearer auth) |
| | `enable_s3_event` | Enables `/api/hooks/s3` |
| `[geocoding]` | `enabled` / `provider` / `mapbox_token` / `nominatim_base_url` / `language` / `cache_precision` | Reverse-geocode GPS to city/country, written to `location`; disabled by default. With `provider=auto`, uses Mapbox when a token is set, otherwise Nominatim |

Minimal example (local photos):

```toml
[server]
listen = "0.0.0.0:8080"
workdir = "./data"
dist_dir = "./web/dist"

# admin_token = "adm123" # Optional

[site]
title = "My Gallery"

[storage.local]
base_path = "./photos"
base_url = "/photos"
```

Online configuration (`/admin`): after setting `[server].admin_token`, edit the config online at `http://<host>:8080/admin` with hot reload — everything takes effect immediately except `listen` (which requires a restart). The underlying API is `GET` / `PUT /api/admin/config` (Bearer admin token, since the config contains secrets).

## Usage

The CLI provides two subcommands:

```bash
afilmory-lite build --config afilmory.toml          # run one build (incremental; --force for a full rebuild)
afilmory-lite serve --config afilmory.toml          # start the resident service (runs one incremental build on startup)
```

## Deployment

- **Docker Compose (recommended)**: see Quick start. Update the image: `docker compose pull && docker compose up -d`.
- **Release tarball**: download `afilmory-lite-<target>.tar.gz` from [Releases](https://github.com/kylinholmes/afilmory-lite/releases), containing the program, `afilmory.example.toml`, and `web/dist/`. After extracting, point `dist_dir` at `web/dist`, finish the configuration, and run `./afilmory-lite serve --config afilmory.toml` (can be kept resident with systemd).

Release artifacts:

- Docker image: `ghcr.io/kylinholmes/afilmory-lite` (amd64 / arm64 multi-arch).
- Tarball: generated by CI and attached to the GitHub Release when a `v*` tag is pushed.

## Feature status

| Capability | Status |
|---|---|
| Builder core (storage → manifest + thumbnails) | ✅ |
| Server and four trigger types (resident daemon) | ✅ |
| S3 and S3-compatible storage (hand-written SigV4) | ✅ |
| Local directory storage (incl. original hosting) | ✅ |
| HDR / Live Photo / Motion Photo | ✅ |
| HEIC (`heic` feature / libheif ≥ 1.18) | amd64 ✅ (enabled in CI and the runtime image via the strukturag PPA); arm64 not yet supported |

Design docs live under [`docs/`](docs/) (upstream feature inventory and per-stage specs/plans).

## Upstream and license

- This project (afilmory-lite) is licensed under the [MIT License](LICENSE).
- The static frontend shell is pulled from the upstream Afilmory repository and compiled at build time; this project does not modify its source.
- An `afilmory-main/` directory in the repo (if present) is a read-only copy of upstream, updated via `pull` and not tracked in version control.
- For Afilmory's license, see its [upstream repository](https://github.com/Afilmory/Afilmory).
