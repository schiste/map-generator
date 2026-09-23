# Hosting on Toolforge

The API ([api.md](api.md)) and the WebAssembly playground run on Wikimedia Toolforge as the
[`map-generator`](https://toolsadmin.wikimedia.org/tools/id/map-generator) tool:
<https://map-generator.toolforge.org/>.

## How it's laid out

| What | Where | Made by |
| --- | --- | --- |
| Server image | Toolforge Build Service, from this repository (Rust buildpack, `Procfile`) | `toolforge build start` |
| Webservice | `toolforge/service.template`: buildservice, NFS mounted, health check `/api/v1/health`, 1 CPU / 2 GiB | `webservice restart` |
| Data | `/data/project/map-generator/data/releases/<UTC time>/`, with `data/current` → the live one | `scripts/prepare-server-data.sh` + `deploy-toolforge.sh --data` |
| Cache | `/data/project/map-generator/cache` (bounded, 3 GB) | the server |
| Playground | `/data/project/map-generator/www`, served at `/` | `deploy-toolforge.sh --www` |

Settings reach the server as Toolforge environment variables (`toolforge envvars list`):
`MAPGEN_DATA_DIR`, `MAPGEN_CACHE_DIR`, `MAPGEN_WWW_DIR`, `MAPGEN_MAX_CONCURRENT` and
`MAPGEN_CACHE_MAX_BYTES`. `deploy-toolforge.sh --restart` sets them.

## Deploying

Every step is a dry run until you add `--apply`. The script runs tool commands with
`become map-generator` over SSH, like Maphue's deploy script, so you need a Toolforge shell
account that maintains the tool.

```sh
# 1. Data: download (pinned), convert and index, then upload as a new release.
#    Only changed files are sent; the switch to the new release is atomic.
scripts/prepare-server-data.sh target/server-data --worldviews all --geoboundaries ADM1
scripts/deploy-toolforge.sh --data target/server-data            # dry run
scripts/deploy-toolforge.sh --data target/server-data --apply

# 2. Server: build the image from GitHub main, then (re)start the webservice.
scripts/deploy-toolforge.sh --build main --restart --apply

# 3. Playground: build the WebAssembly package and upload it, with its own copy of
#    the Natural Earth files, so visitors' browsers contact no third party.
scripts/deploy-toolforge.sh --www --apply

# 4. Pre-render every region's default map into the cache.
scripts/warm-cache.sh https://map-generator.toolforge.org/api/v1
```

The data is reproducible: the downloads are pinned (Natural Earth commit, geoBoundaries
release ids in the licence sidecars), and `/api/v1/version` reports the release of each dataset.

## Operating

- **Logs.** `ssh login.toolforge.org`, `become map-generator`, then
  `toolforge webservice buildservice logs -f`. There is one line per request (method, path,
  status, time, cache hit), with no IP addresses or user agents.
- **Health.** <https://map-generator.toolforge.org/api/v1/health> (the webservice health check).
- **Rollback.** Data: point `data/current` back at the previous release
  (`ln -sfn releases/<older> current.new && mv -Tf current.new current`), then restart. The
  three newest releases are kept. Code: `scripts/deploy-toolforge.sh --build <tag> --restart --apply`.
- **Cache.** It is keyed by mapgen version and dataset release, so a new release or version
  never serves stale maps. `rm -rf cache/*` is always safe.
- **Resources.** `toolforge webservice buildservice status`. If renders queue, raise `cpu`/`mem`
  in `toolforge/service.template` (within the tool's quota) and `MAPGEN_MAX_CONCURRENT`.

## Policies

- [Wikimedia Cloud Services Terms of Use](https://wikitech.wikimedia.org/wiki/Wikitech:Cloud_Services_Terms_of_use):
  the code is open source (MIT), nothing identifies visitors, and no personal data is stored.
- The playground and the API load nothing from third parties.
- Each map carries its data's credit and licence ([api.md](api.md#terms)).
