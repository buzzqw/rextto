# Rextto

**Rextto** è un demone self-hosted per l'acquisizione e l'archiviazione automatica
di serie TV, film e fumetti.

Un solo processo Rust racchiude tutto: motore di scraping, archivio SQLite, web
UI/API e una sessione **libtorrent** integrata. Non servono servizi esterni.

Monitora ciò che configuri, cerca su feed RSS/HTML, indexer Torznab
(Jackett/Prowlarr) e motori di ricerca pubblici, assegna un punteggio di qualità a
ogni release, scarica la migliore e la rinomina/archivia nella tua libreria (NAS
o disco locale).

> 🇬🇧 English: [`README.md`](README.md)
> 📖 Manuale completo: [`docs/MANUAL.it.md`](docs/MANUAL.it.md) ·
> [`docs/MANUAL.en.md`](docs/MANUAL.en.md)

[![Dona](https://img.shields.io/badge/❤️_Sostieni_Rextto-PayPal-00457C.svg)](https://www.paypal.com/cgi-bin/webscr?cmd=_donations&business=azanzani@gmail.com&item_name=Support+Rextto+Project)

---

## Cos'è Rextto

- **Un demone, nessun orchestratore esterno** — scraping, download, rinomina,
  archiviazione e UI vivono nello stesso processo.
- **Sorgenti multiple** — feed RSS generici, listing HTML (con fallback
  FlareSolverr per Cloudflare), indexer Torznab (Jackett/Prowlarr) e motori web.
- **Punteggio qualità** — risoluzione, sorgente, codec, audio, HDR/Dolby Vision,
  gruppi e pesi configurabili, con simulatore integrato.
- **Upgrade automatici** — sostituisce un file archiviato quando compare una
  release migliore (salto di risoluzione, HDTV→WEB-DL, HDR, repack) oltre una
  soglia di punteggio configurabile.
- **Serie e film** — metadati TMDB, locandine, monitoraggio per stagione, ricerca
  episodi mancanti, calendario, ricerca manuale.
- **Torrent** — libtorrent embedded: coda, limiti, tag, peer, tracker, file,
  spostamento storage, politica di seeding, fastresume, killswitch VPN.
- **Fumetti** — monitoraggio GetComics e weekly pack.
- **Integrazioni** — Trakt, Simkl, Jellyfin, Plex, notifiche Telegram/e-mail/webhook.
- **UI web** — single-page responsive, tema chiaro/scuro, **italiano e inglese**,
  con log viewer, salute, grafici e manutenzione.
- **Visti dai feed** — ogni release vista nelle sorgenti, raggruppata per titolo,
  consultabile anche per ciò che non è monitorato.
- **Backup** — manuali o programmati (locale, FTP, cartella cloud, Telegram).

## Installazione

### Requisiti

- Linux, Rust stable (`rustc`/`cargo`).
- Header di sviluppo di `libtorrent-rasterbar` e compilatore C++17 (il bridge è
  compilato da `build.rs`), più gli header OpenSSL.
- Opzionali: `mediainfo` (tag tecnici per la rinomina), `mold` (link più rapido),
  FlareSolverr (sorgenti protette da Cloudflare).
- Per la UI: `cargo-leptos` e il target `wasm32-unknown-unknown`.

```bash
sudo apt-get install -y build-essential libtorrent-rasterbar-dev libssl-dev mediainfo
```

### 1. Compila il demone

```bash
cargo build --release
# binario: target/release/rexttod
```

### 2. Compila la UI web

La UI è un bundle statico servito dal demone.

```bash
rustup target add wasm32-unknown-unknown
cargo install cargo-leptos
cargo leptos --manifest-path ui/Cargo.toml build --release --frontend-only
```

### 3. Prima prova in dry-run

Non parte nessun download reale e non tocca file fuori dalla propria directory.

```bash
./run-safe.sh          # avvia su http://127.0.0.1:5000 con REXTTO_ACTIVE=0
```

### 4. Installa come servizio

Lo script compila se serve, installa/aggiorna l'unità systemd e riavvia.

```bash
./start-rextto-service.sh
sudo systemctl status rextto.service
sudo journalctl -u rextto.service -f
```

### 5. Verifica

```bash
curl --fail http://127.0.0.1:5000/api/status
curl --fail http://127.0.0.1:5000/api/health
```

## Come si usa

Tutto si gestisce dalla UI web, all'indirizzo configurato (default
`http://<host>:5000`).

### Primo avvio

1. Apri la UI. Se non esiste ancora una data directory, completa il **setup
   iniziale**.
2. Lascia il daemon in **dry-run** finché non hai configurato le sorgenti: in
   dry-run i download non partono.
3. Quando sei pronto, abilita la **modalità attiva** in
   *Configurazione → Daemon*.

### Configura le sorgenti

In *Configurazione → Sorgenti* aggiungi:

- feed RSS / listing HTML (URL e pagine da seguire);
- indexer Torznab (Jackett/Prowlarr) con pulsante **Verifica**;
- motori di ricerca web e, se serve, l'URL di FlareSolverr;
- filtri contenuto e blacklist.

Nella stessa sezione regoli punteggi, rinomina, percorsi e libtorrent.

### Aggiungi serie e film

- Da **Esplora** (ricerca TMDB) con un clic, oppure
- da **Serie TV / Film → Aggiungi** in manuale.

Per ogni titolo scegli qualità minima, lingua, stagioni/anni, alias, esclusioni e
**percorso NAS**. I fumetti si gestiscono da **Fumetti**.

### Cicli e download

Rextto lavora a cicli: cerca, valuta, scarica, rinomina e archivia.

- Dalla **Dashboard** avvii un ciclo completo, di un solo dominio (Serie, Film,
  Fumetti) o un backup immediato.
- I cicli girano anche in automatico all'intervallo configurato.
- In **Scarico → Sessione torrent** vedi i torrent nel client (con badge **NAS** se
  già archiviati); sotto il nome trovi il **motivo** del download e la **fonte**
  (indexer/RSS/web). **Pulisci completati** rimuove i torrent che hanno raggiunto
  il limite di seed.
- In **Storico download** finiscono i torrent **usciti dalla sessione**, con esito
  (NAS o motivo dell'eventuale scarto).

### Le sezioni della UI

| Sezione | A cosa serve |
|---|---|
| **Dashboard** | Ricerca manuale, avvio cicli, statistiche, rete, prossime uscite |
| **Scarico** | Sessione torrent, aggiunta magnet/.torrent, storico |
| **Serie TV / Film** | Libreria, dettagli, episodi mancanti, ricerca manuale |
| **Mancanti** | Episodi mancanti e riempimento |
| **Calendario** | Prossime uscite dalle serie monitorate |
| **Esplora** | Scoperta TMDB e ricerca release |
| **Archivio** | Release passate; *Visti dal feed* per film/serie |
| **Fumetti** | GetComics e weekly pack |
| **Configurazione** | Sorgenti, libtorrent, punteggi, rinomina, percorsi, notifiche |
| **Integrazioni** | Trakt, Simkl, Jellyfin, Plex |
| **Manutenzione** | Backup, duplicati, scoring, import legacy, riavvio |
| **Salute / Log / Grafici** | Diagnostica e monitoraggio |

La guida dettagliata di ogni schermata è nel
[manuale](docs/MANUAL.it.md).

### Terminal UI

Per i server senza browser, Rextto include una TUI in Python senza dipendenze
(solo libreria standard `curses` + `urllib`, niente da installare):

```bash
python3 scripts/rextto_tui.py
# oppure verso un'altra istanza:
REXTTO_URL=http://192.168.1.10:5000 REXTTO_API_TOKEN=... python3 scripts/rextto_tui.py
```

Schede: **Status · Torrents · Logs · Health** (auto-refresh).

| Tasti | Azione |
|---|---|
| `1-4` / `Tab` | cambia scheda |
| `↑↓` | seleziona un torrent nella scheda Torrents |
| `Invio` | apre i dettagli del torrent selezionato (`Invio`/`Esc` per tornare) |
| `r` | aggiorna · `c` avvia un ciclo · `q` esci |
| `a` | **aggiungi un magnet** |
| `t` | **aggiungi un file `.torrent`** (percorso) |
| `p` | pausa/riprendi il torrent selezionato |
| `d` | rimuovilo (chiede se eliminare anche i file) |
| `k` / `R` | recheck / reannounce |
| `n` | attiva/disattiva "non rinominare" |
| `x` | nella scheda Health, pulisce il trash (chiede conferma) |

### Dati e log

- Data directory di default: `data/` (modificabile con `REXTTO_DATA_DIR`).
- Log in `data/rextto.log`, con rotazione a 5 MB (file attivo + 3 backup),
  consultabili in streaming dalla UI.

### Variabili d'ambiente

| Variabile | Scopo |
|---|---|
| `REXTTO_DATA_DIR` | Data directory (database, log, download) |
| `REXTTO_LISTEN` | Indirizzo UI/API (default `0.0.0.0:5000`) |
| `REXTTO_ENGINE_LISTEN` | Canale interno del motore (default `127.0.0.1:8889`) |
| `REXTTO_ACTIVE` | `1` abilita i cicli di acquisizione |
| `REXTTO_DRY_RUN` | `1` disabilita i download reali |
| `REXTTO_API_TOKEN` | Token bearer opzionale per API/UI |
| `RUST_LOG` | Filtro tracing (default `rextto=info`) |
| `REXTTO_URL` | Client TUI: URL base del demone (default `http://127.0.0.1:5000`) |

## Sviluppo

```bash
./scripts/build-fast.sh          # build rapida del demone (target/fast/rexttod)
./scripts/dev-reload.sh --daemon # compila + copia + riavvia il servizio
./scripts/dev-reload.sh          # demone + UI + riavvio
cargo check                      # feedback più rapido
cargo test --all-targets         # test
```

Le build di sviluppo restano piccole e non crescono all'infinito:

- I profili `dev`/`test` usano solo tabelle di riga e nessuna info di debug per le
  dipendenze (`Cargo.toml`): `target/debug` resta intorno a 1 GB invece di ~10 GB.
- `scripts/clean.sh` mostra lo spazio recuperabile; `--debug` rimuove gli
  artefatti di sviluppo (mantenendo il binario release e il bundle UI servito).
- Un timer systemd **utente** settimanale esegue `scripts/clean.sh --auto` (salta
  se una build è in corso). Si installa una volta con:

```bash
scripts/install-dev-clean-timer.sh
```

## Migrazione da un'istanza precedente

Un importer opzionale legge una data directory storica ferma (database serie,
archivio, fumetti) e la copia nei file `rextto_*.db`. Non scrive mai nella
directory sorgente.

```bash
./import-legacy.sh /percorso/legacy /home/user/rextto/data
```

## ❤️ Sostieni il progetto

Rextto è software libero e open-source, costruito interamente nel tempo libero.
Se ti fa risparmiare ore di configurazione, RAM o usura del disco, considera di
offrire un caffè all'autore.

Ogni donazione finanzia direttamente nuove funzionalità, correzioni di bug e la
sopravvivenza del progetto.

<div align="center">

[![Dona con PayPal](https://img.shields.io/badge/Dona-PayPal-00457C?style=for-the-badge&logo=paypal)](https://www.paypal.com/cgi-bin/webscr?cmd=_donations&business=azanzani@gmail.com&item_name=Support+Rextto+Project)

*Grazie. Sul serio.*

</div>

## ⚖️ Uso lecito & responsabilità

Rextto è uno **strumento di automazione dei download**. Non ospita, non indicizza
e non distribuisce alcun contenuto protetto da copyright.

- Rextto si connette agli **indexer che configuri tu** (Jackett, Prowlarr, feed
  RSS pubblici). Non ha un indice integrato.
- Ciò che scarichi è **interamente sotto la tua responsabilità**. Usa Rextto solo
  per contenuti che hai il diritto di accedere — dominio pubblico, licenze
  Creative Commons, o media di tua proprietà.
- L'integrazione torrent (libtorrent) è una tecnologia neutrale. Rextto non
  incoraggia né facilita la pirateria.
- Questo progetto è rilasciato sotto licenza open-source **EUPL 1.2**.

> *"Con grande automazione viene grande responsabilità."*

## Licenza

Rilasciato sotto **European Union Public Licence v. 1.2** — vedi [`LICENSE`](LICENSE).
