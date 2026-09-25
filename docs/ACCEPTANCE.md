# Release checklist

Run this before publishing a build or a release tag.

## 1. Source hygiene

- No data, logs, build artefacts or secrets are tracked:

  ```bash
  git ls-files | grep -Ei '(^|/)(data|target|node_modules)/|\.db($|-)|\.log|build_number|\.env|secret'
  # expected: no output
  ```

- `.gitignore` covers `data/`, `target/`, `ui/target/`, `*.db*`, `*.log*`,
  `build_number`, `.env*`.
- No API keys, tokens or personal paths in tracked files.

## 2. Build

```bash
cargo build --release
rustup target add wasm32-unknown-unknown
(cd ui && cargo leptos build --frontend-only)
```

## 3. Tests

```bash
cargo test --all-targets
```

All tests must pass. The suite covers parser/quality/scoring, database and
migrations, archive search, import, RSS/listing parsing, media tags and
filename building, log rotation, notifier formatting, config and API auth.

## 4. Runtime smoke test (dry-run)

```bash
REXTTO_DATA_DIR="$PWD/data-acceptance" REXTTO_ACTIVE=0 REXTTO_DRY_RUN=1 \
  cargo run --release -- --dry-run
curl --fail http://127.0.0.1:5000/api/status
curl --fail http://127.0.0.1:5000/api/health
```

Check, in order:

- the UI loads (dashboard, settings, health, logs);
- a manually triggered cycle runs and ends with a `CYCLE REPORT` line;
- Settings → Sources shows the configured feeds/indexers;
- Maintenance → Database VACUUM/ANALYZE runs on **all** databases;
- Maintenance → Backups *Test FTP* reports every step;
- `/feed.xml` returns a valid magnet feed.

## 5. Service install

```bash
curl -fsSL https://raw.githubusercontent.com/buzzqw/rextto/main/install.sh | bash
sudo systemctl status rextto.service
sudo journalctl -u rextto.service -f
```

The installer creates the dedicated service account, runtime directories,
systemd unit and initial databases.

## 6. Enable real downloads

Only after the checks above, set `REXTTO_ACTIVE=1` / `REXTTO_DRY_RUN=0` and make
sure no other service is using the web/engine ports.

## 7. Command line and self-update

```bash
target/release/rexttod --version   # daemon + build + libtorrent
target/release/rexttod --help
```

`--update` must preserve the data directory and stay atomic. Test it against a
local archive and a throwaway install directory:

```bash
scripts/package-linux.sh \
  --binary target/release/rexttod --ui ui/target/site \
  --output /tmp/rextto-linux-x86_64.tar.gz
mkdir -p /tmp/rextto-update-check
target/release/rexttod --update \
  --archive /tmp/rextto-linux-x86_64.tar.gz \
  --install-dir /tmp/rextto-update-check --no-restart
/tmp/rextto-update-check/rexttod --version
```

The archive must contain `rexttod`, `ui/pkg/ui.js`, `lib/libtorrent-rasterbar.so.2.0`
and `run.sh`:

```bash
tar -tzf /tmp/rextto-linux-x86_64.tar.gz
```

Finally, extract it in a clean machine or container **without** libtorrent
installed and confirm `./run.sh --version` and a dry-run start work, proving the
bundled library and UI are complete.
