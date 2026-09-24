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
