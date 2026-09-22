# Rextto

**Rextto** è un demone self-hosted per l'acquisizione e l'archiviazione di media.
Un unico processo Rust integra il motore di scraping, l'archivio SQLite, la web
API/UI e una sessione libtorrent embedded: non servono servizi esterni per farlo
funzionare.

Monitora le serie e i film configurati, cerca su feed RSS/HTML, indexer Torznab
(Jackett/Prowlarr) e motori di ricerca pubblici, assegna un punteggio di qualità
a ogni release, scarica la migliore e la rinomina/archivia nella libreria.

> 🇬🇧 English: see [`README.md`](README.md).
> Manuale completo: [`docs/MANUAL.it.md`](docs/MANUAL.it.md) ·
> [`docs/MANUAL.en.md`](docs/MANUAL.en.md).

---

## Funzionalità

- **Sorgenti** — feed RSS generici, listing HTML (con fallback FlareSolverr per
  Cloudflare), indexer Torznab (Jackett/Prowlarr) e motori di ricerca web.
- **Punteggio qualità** — risoluzione, sorgente, codec, audio, HDR/Dolby Vision,
  gruppi e pesi configurabili, con simulatore integrato.
- **Upgrade automatici** — sostituisce un file archiviato quando compare una
  release migliore (salto di risoluzione, HDTV→WEB-DL, HDR, repack) più soglia
  di punteggio configurabile.
- **Serie e film** — metadati TMDB, locandine, monitoraggio per stagione, ricerca
  episodi mancanti, calendario, ricerca manuale.
- **Torrent** — libtorrent embedded: coda, limiti per torrent, tag, peer,
  tracker, file, spostamento storage, politica di seeding, fastresume.
- **Fumetti** — monitoraggio GetComics e weekly pack.
- **Integrazioni** — Trakt, Simkl, Jellyfin, Plex, notifiche Telegram/e-mail/
  webhook.
- **Privacy** — killswitch VPN: vincola ascolto e traffico in uscita di
  libtorrent a un'interfaccia scelta (`tun0`/`wg0`), selezionabile dalla UI.
- **UI web** — single-page responsive (tema chiaro/scuro) con localizzazione
  **italiano e inglese**; log viewer, salute, grafici, manutenzione.
- **Visti dai feed** — ogni release vista nelle sorgenti, raggruppata per
  titolo e consultabile in *Archivio → Visti dai feed*, anche se non monitorata.
- **Manutenzione e backup** — pulizia duplicati video, ripristino del token
  sorgente perso, prune dei database e backup manuali/programmati (locale, FTP,
  cartella cloud, Telegram).
- **Feed** — feed magnet RSS rolling su `/feed.xml`, per consumer esterni.

## Requisiti

- Linux, Rust stable (`rustc`/`cargo`).
- Header di sviluppo di `libtorrent-rasterbar` e compilatore C++17 (il bridge
  libtorrent è compilato da `build.rs`), più gli header OpenSSL.
- Opzionali: `mediainfo` (tag tecnici per la rinomina), `mold` (link più rapido),
  FlareSolverr (sorgenti protette da Cloudflare), `cargo-leptos` + target
  `wasm32-unknown-unknown` (per compilare la UI).

```bash
sudo apt-get install -y build-essential libtorrent-rasterbar-dev libssl-dev mediainfo
```

## Compilazione

```bash
# Demone (ottimizzato, usato dall'unità systemd)
cargo build --release

# UI web (bundle statico servito dal demone)
rustup target add wasm32-unknown-unknown
cargo install cargo-leptos
cd ui && cargo leptos build --frontend-only
```

Per lo sviluppo c'è un profilo ancora più veloce (minore ottimizzazione) e degli
script:

```bash
./scripts/build-fast.sh          # target/fast/rexttod
./scripts/dev-reload.sh --daemon # compila + copia + riavvia il servizio
./scripts/dev-reload.sh          # demone + UI + riavvio
cargo check                      # feedback più rapido
```

## Configurazione

Rextto legge `rextto.json` (se presente) e salva le impostazioni in
`rextto_config.db`. La data directory è `data/` e si può cambiare con le
variabili d'ambiente:

| Variabile | Scopo |
|---|---|
| `REXTTO_DATA_DIR` | Data directory (database, log, download) |
| `REXTTO_LISTEN` | Indirizzo UI/API (default `0.0.0.0:5000`) |
| `REXTTO_ENGINE_LISTEN` | Canale interno del motore (default `127.0.0.1:8889`) |
| `REXTTO_ACTIVE` | `1` abilita i cicli di acquisizione |
| `REXTTO_DRY_RUN` | `1` disabilita i download reali |
| `REXTTO_API_TOKEN` | Token bearer opzionale per API/UI |
| `RUST_LOG` | Filtro tracing (default `rextto=info`) |

Quasi tutte le opzioni sono modificabili da **Configurazione** nella UI
(sorgenti, indexer, punteggi, libtorrent, rinomina, notifiche, percorsi,
traduzioni).

## Avvio

Dry-run (scrive solo nella propria directory):

```bash
./run-safe.sh
```

Come servizio:

```bash
./start-rextto-service.sh        # installa/aggiorna l'unità systemd e riavvia
sudo systemctl status rextto.service
sudo journalctl -u rextto.service -f
```

Controlli rapidi a servizio attivo:

```bash
curl --fail http://127.0.0.1:5000/api/status
curl --fail http://127.0.0.1:5000/api/health
```

## UI e API

La UI è servita da `ui/target/site/pkg` sull'indirizzo configurato. Endpoint
principali:

- `GET /api/status`, `GET /api/health`
- `GET /api/config`, `POST /api/config/settings`, `POST /api/config/library`
- `GET /api/series`, `GET /api/movies`, `GET /api/gaps`, `GET /api/calendar`
- `GET /api/torrents`, `POST /api/send-magnet`, azioni torrent sotto
  `/api/torrents/{hash}/...`
- `GET /api/archive`, `POST /api/archive/batch-download`
- `GET /api/series/seen/grouped`, `GET /api/movies/seen/grouped` (visti dai feed)
- `GET /api/network/interfaces`
- `POST /api/maintenance/clean-duplicates`, `POST /api/maintenance/restore-source`
- `GET /api/comics`, `POST /api/comics/explore`
- `POST /api/search`, `GET /api/sources/health`
- `GET /api/logs/stream` (SSE), `GET /feed.xml` (feed magnet RSS)

## Log

I log sono in `data/rextto.log` con **rotazione a 5 MB**, mantenendo il file
attivo più tre backup (`rextto.log.1` … `rextto.log.3`). Il viewer nella UI li
mostra in streaming.

## Test

```bash
cargo test --all-targets
```

## Migrazione da un'istanza precedente

Un importer opzionale legge una data directory storica ferma (database serie,
archivio, fumetti) e la copia nei file `rextto_*.db`:

```bash
./import-legacy.sh /percorso/legacy /home/user/rextto/data
```

Non scrive mai nella directory sorgente.

## Licenza

Rilasciato sotto **European Union Public Licence v. 1.2** — vedi
[`LICENSE`](LICENSE).
