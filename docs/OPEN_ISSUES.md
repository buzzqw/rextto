# Rextto — Problemi aperti (handoff per un'altra AI)

Documento di passaggio: descrive i problemi **ancora aperti**, con evidenze, file
coinvolti e prossimi passi. Aggiornato al commit `1c58faf` (branch `main`,
pushato su `origin`).

## Contesto rapido

- Progetto: **Rextto**, daemon Rust/Axum (`rexttod`) per serie/film/fumetti con
  libtorrent; persistenza SQLite.
  - Backend: `src/` (in particolare `src/web.rs`, `src/orchestrator.rs`,
    `src/engine.rs`, `src/rss.rs`, `src/libtorrent.rs`, `src/archive.rs`,
    `src/postprocess.rs`).
  - UI: Leptos/WASM in `ui/app/src/lib.rs` + SCSS `ui/style/main.scss`; il bundle
    servito è `ui/target/site/pkg` (build con
    `cargo leptos --manifest-path ui/Cargo.toml build --release --frontend-only`).
- Dati runtime: `data/` con `rextto_archive.db` (~389k righe),
  `rextto_series.db`, `rextto_config.db`, `rextto.log`.
- Legacy di import: `/home/andres/extto` (`extto_archive.db`, `extto3.py`).
- Servizio: systemd `rextto`, binario `target/release/rexttod`. Riavvio senza
  password: `sudo -n /usr/local/bin/rextto-restart restart` (script che fa
  `systemctl --no-block restart rextto.service`).
- Mount NFS: `/home/andres/trasferimento` e l'archivio su `192.168.1.119`
  (NFS4, `hard`). Le operazioni su file possono essere lente.
- FlareSolverr configurato a `http://192.168.1.161:8191/` ma **irraggiungibile**
  (fallisce con "error sending request"). I feed dietro Cloudflare (ext.to)
  dipendono da FlareSolverr.
- Profilo `release` in `Cargo.toml` ha `strip = true` → il binario non ha
  simboli. Per diagnosticare usare:
  `CARGO_PROFILE_RELEASE_DEBUG=line-tables-only CARGO_PROFILE_RELEASE_STRIP=false cargo build --release`
  e poi `gdb -batch -p <pid> -ex "thread apply all bt"`.

---

## PROBLEMA 1 (CRITICO, aperto) — Il daemon diventa non responsivo durante il ciclo

### Sintomo
Mentre il ciclo automatico è nella fase **Step 2/2 (ricerca di ~91 titoli su
indexer Torznab + motori web)** le API HTTP smettono di rispondere:

- `curl http://127.0.0.1:5000/api/health` → timeout (`000`), idem `/api/torrents`.
- Il TCP viene accettato dal kernel (porta LISTEN con backlog, connessione
  `ESTABLISHED`) ma il server non risponde → il task di accept/handler non gira.
- Il log si ferma su:
  `🔎 Step 2/2: searching 91 series/movies (Torznab indexers + web engines)`
  e non arriva più nulla (nemmeno i log `📊 Queue` del loop principale).
- Il riavvio è lento: SIGTERM non viene gestito (event loop bloccato) e systemd
  arriva al SIGKILL dopo il timeout (~90 s).

### Ricorrenza
Osservato più volte, a ~2-4 minuti dall'avvio, **anche senza richieste manuali**;
coincide con l'avvio di Step 2/2. Si ripresenta a ogni riavvio nel momento in cui
parte quella fase.

### Evidenze raccolte
- `strace -f -p <pid>`: un solo thread in `epoll_wait` molto attivo (I/O di rete,
  molti socket), gli altri in `futex`. Non c'è attesa su file system NFS.
- fd aperti: 90-118 su un limite di 1024 → **non** è saturazione di fd.
- CPU del processo ~25-30% di un core (non è CPU-saturato, ma non è nemmeno un
  deadlock totale).
- `gdb` sul binario release: frame `??` (nessun simbolo) → inutile senza rebuild
  con debug.
- `/proc/<pid>/task/*/wchan`: `futex_do_wait` per quasi tutti i thread + uno
  `do_epoll_wait`.

### Ipotesi
1. Lavoro CPU-bound (parsing HTML/JSON delle decine di ricerche) eseguito **sul
   runtime async** senza `spawn_blocking`/yield: con 4 worker (`nproc`) satura il
   runtime e affama l'accept di hyper.
2. Un lock `std::sync::Mutex` (es. `AppState.db`, `AppState.torrents`) tenuto
   attraverso un `.await` durante la fase di ricerca, che blocca i worker.
3. Il client HTTP che si satura / il pool di connessioni che non rilascia.

### Prossimi passi consigliati
1. **Backtrace simbolizzato** durante lo stallo:
   ```
   CARGO_PROFILE_RELEASE_DEBUG=line-tables-only CARGO_PROFILE_RELEASE_STRIP=false \
     cargo build --release
   sudo -n /usr/local/bin/rextto-restart restart        # attende ~100 s
   # appena /api/health non risponde:
   gdb -q -batch -p "$(pgrep -f target/release/rexttod | head -1)" \
       -ex "set pagination off" -ex "thread apply all bt" > /tmp/bt.txt
   ```
   Il thread che *non* è in `futex`/`epoll` e il suo stack diranno se è CPU,
   lock o I/O.
2. Aggiungere log temporanei con timing attorno a `engine.search_query` /
   `run_cycle_domain` Step 2/2 (inizio/fine per titolo) e controllare se il
   thread di accept resta affamato.
3. In `src/orchestrator.rs` Step 2/2 (`run_cycle_domain`, ~righe 316-347) la
   ricerca per titolo è `engine.search_query(&cfg, &query).await`: verificare se
   dentro `src/engine.rs` / `src/websearch.rs` c'è lavoro sincrono pesante su un
   worker, e spostarlo su `tokio::task::spawn_blocking`.
4. Verificare che nessun `MutexGuard` (`db.lock()`, `torrents.read()/write()`)
   sia tenuto attraverso un `.await` in `web.rs`/`orchestrator.rs`.
5. Valutare un timeout/limite di concorrenza sulla scansione dei 91 titoli e
   l'uso di `tokio::task::yield_now()` nei loop di parsing.

### File coinvolti
- `src/orchestrator.rs` (`run_cycle_domain`, Step 2/2)
- `src/engine.rs` (`search_query`, `search_query_ids`, `search_one`)
- `src/websearch.rs` (`search`, `search_with_timeout`)
- `src/web.rs` (event loop `torrent_event_worker`, handler HTTP, `AppState`)

---

## PROBLEMA 2 (risolto, da verificare) — Parser ext.to e magnet civetta

### Causa individuata
`extract_magnet` in `src/rss.rs` include un fallback che accetta **qualsiasi
token di 32 caratteri base32** presenti nella pagina:
```rust
crate::utils::cached_regex(r#"(?i)\b([a-f0-9]{40}|[a-z2-7]{32})\b"#)
```
ext.to, per alcune voci, serve un magnet **civetta anti-bot** con
`btih:areMouseMovesMostlyStraightLined`. Il fallback lo scambiava per un
infohash base32 e lo archiviava come magnet (presente in 89 righe
dell'archivio, tutte sorgente `ExtTo`).

### Fix applicato (commit `6b5132d`)
- nuova `is_placeholder_magnet()` in `src/rss.rs`;
- guard in `push_release_at`, in `extract_magnet` e nella cache
  (`fetch_detail_magnets`: una civetta in cache è trattata come assente e si
  risolve dal dettaglio).

### Da verificare
- Un nuovo scrape reale di ext.to non deve più produrre placeholder. **Non
  testabile ora** perché FlareSolverr è giù (ext.to risponde 403 diretto).
- Pulizia dati fatta: eliminate le 89 righe civetta dall'archivio (0 residue),
  cache magnet ripulita (891 → 651).

---

## PROBLEMA 3 (parzialmente risolto) — URL `.torrent` dei feed Jackett

### Contesto
L'archivio contiene ~208k voci con **URL `http(s)://` `.torrent`** invece di
magnet. Sono soprattutto feed "Jackett RSS - *". I risultati Prowlarr e le
sorgenti ExtTo/Knaben/ThePirateBay/Corsaro/TGx archiviano magnet validi.

Gli URL Jackett del tipo
`http://192.168.1.161:9117/dl/<indexer>/?jackett_apikey=<KEY>&path=<base64>`
sono **ephemerali**: testandone uno salvato in archivio si ottiene **404**.
(La chiave `jackett_apikey` è un segreto: non riportarla nei log/doc.)

### Fix applicato
- commit `45a8a59`: l'Accoda manuale (`src/web.rs`, `add_release` /
  `add_raw_magnet`) riconosce gli URL `http(s)`, scarica il `.torrent` e lo
  aggiunge a libtorrent (`LibtorrentClient::add_torrent_file_with_path`,
  aggiunto in `src/libtorrent.rs`), registrando i metadati sotto l'hash reale.
- commit `1c58faf`: se il download dell'URL fallisce (404), `resolve_by_search`
  cerca la release per titolo con `Engine::search_query_manual` e aggiunge il
  primo risultato con sorgente usabile (magnet o URL), richiedendo che il titolo
  normalizzato condivida la radice (query = prime parole fino a
  qualità/anno, es. "Signal One 2026").

### Da verificare / completare
- Il fallback **non è stato confermato dal vivo**: durante i test il daemon era
  in stallo (Problema 1), quindi la ricerca non rispondeva. Da ritestare quando
  il Problema 1 è risolto:
  ```
  curl -X POST http://127.0.0.1:5000/api/archive/add \
    -H 'content-type: application/json' \
    -d '{"title":"<titolo archivio>","magnet":"<url .torrent>","source":"archive"}'
  ```
- Valutare, in alternativa/meglio: all'**ingest** dei feed Jackett RSS,
  estrarre il magnet dalla pagina di dettaglio invece di salvare l'URL
  `.torrent`, così l'archivio contiene sorgenti durature (come fa Prowlarr).

---

## Audit delle sorgenti (fatto)

Analisi di `rextto_archive.db` per sorgente (classificazione del campo `magnet`):

| Sorgente (prefisso)        | Totale | magnet 40-hex | magnet base32 | http URL | altro |
|----------------------------|-------:|--------------:|--------------:|---------:|------:|
| Jackett RSS - LimeTorrents |  98433 |             0 |             0 |    98433 |     0 |
| Jackett RSS - TorrentGalaxy|  80886 |             0 |             0 |    80886 |     0 |
| Prowlarr - LimeTorrents    |  69704 |         69704 |             0 |        0 |     0 |
| Prowlarr - Knaben          |  36116 |         35900 |           216 |        0 |     0 |
| ExtTo                      |  27670 |         27670 |             0 |        0 |     0 |
| Jackett RSS - 1337x        |  10227 |             0 |             0 |    10227 |     0 |
| Jackett RSS - Knaben       |   8269 |          8210 |             0 |       59 |     0 |
| Knaben                     |   7698 |          7697 |             1 |        0 |     0 |
| ThePirateBay               |   7671 |          7671 |             0 |        0 |     0 |
| Jackett (generico)         |   4802 |           241 |             0 |     4561 |     0 |
| Jackett - LimeTorrents     |   4508 |             0 |             0 |     4508 |     0 |
| Corsaro                    |   1433 |          1433 |             0 |        0 |     0 |

Conclusioni:
- **Nessun magnet con btih invalido** (verificato: 0 righe con `btih` che non
  sia 40-hex o 32-base32).
- **Nessuna civetta residua** (dopo la pulizia).
- Le uniche sorgenti "deboli" sono i **feed Jackett RSS** (URL `.torrent`
  ephemerali); Prowlarr e le altre danno magnet validi.
- Nota: i "magnet base32" (32 caratteri `[A-Z2-7]`) sono **legittimi**
  (SubsPlease, Erai-raws, ecc.), non vanno scambiati per corrotti.

---

## Altri punti aperti / note

- **`auto_remove_completed = true`** (attivo): rimuove i torrent completati al
  limite di seed (i pack archiviati escono da soli). Fix recente `d70758d`:
  non rimuove mai un download **incompleto** (prima un torrent in pausa a metà
  veniva rimosso).
- **FlareSolverr giù** (`192.168.1.161:8191`): i feed dietro Cloudflare (ext.to)
  falliscono. Il retry diretto (3×20 s) è attivo; l'ultima spiaggia FlareSolverr
  non è disponibile. Da sistemare a livello di rete/servizio, non di codice.
- **`.torrent`/RSS**: valutare se scaricare il `.torrent` all'ingest e salvare il
  magnet (o l'infohash) per i feed che espongono solo il download (Jackett RSS).
- **Cache magnet** (`data/rextto_magnet_cache.json`): ripulita su file; il
  daemon potrebbe riscriverla dalla copia in memoria (le civette sono comunque
  ignorate dal parser).

## Comandi utili

```bash
# stato servizio
systemctl is-active rextto
curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:5000/api/health

# riavvio (richiede l'helper sudoers)
sudo -n /usr/local/bin/rextto-restart restart

# log
tail -40 data/rextto.log

# build + test
cargo test --quiet
cargo build --release
cargo leptos --manifest-path ui/Cargo.toml build --release --frontend-only

# diagnostica stallo (binario con simboli)
CARGO_PROFILE_RELEASE_DEBUG=line-tables-only CARGO_PROFILE_RELEASE_STRIP=false cargo build --release
sudo -n /usr/local/bin/rextto-restart restart
gdb -q -batch -p "$(pgrep -f target/release/rexttod | head -1)" \
    -ex "set pagination off" -ex "thread apply all bt" > /tmp/bt.txt
```

## Commit recenti rilevanti

- `1c58faf` Retry stale .torrent links with an indexer search
- `45a8a59` Queue archive entries that are .torrent URLs
- `6b5132d` Ignore the ext.to anti-bot placeholder magnet
- `d70758d` Never auto-remove an incomplete torrent
- `72bd967` Fix the dynamic queue ceiling, de-prioritize slow downloads, clean seeded packs and readable logs
- `b00d309` Match monitored movies tolerantly when queueing manually
- `9caa3d5` Retry RSS feed fetches on transient truncation
```
