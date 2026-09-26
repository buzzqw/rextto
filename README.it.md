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
- **Autonomo e auto-aggiornante** — un unico archivio di release (demone, UI web,
  libtorrent inclusa) che `rexttod --update` installa in modo atomico,
  preservando database e configurazione.
- **Sorgenti multiple** — feed RSS generici, listing HTML (con fallback
  FlareSolverr per Cloudflare), indexer Torznab (Jackett/Prowlarr) e motori web.
- **Punteggio qualità** — risoluzione, sorgente, codec, audio, HDR/Dolby Vision,
  gruppi e pesi configurabili, con simulatore integrato. Lo stesso punteggio
  viene usato per acquisizione, ricerche, upgrade, post-processing, archivio e
  rescore, includendo bonus dimensione e sottotitoli dei film.
- **Upgrade automatici** — sostituisce un file archiviato quando compare una
  release migliore (salto di risoluzione, HDTV→WEB-DL, HDR, repack) oltre una
  soglia di punteggio configurabile.
- **Serie e film** — metadati TMDB, locandine, monitoraggio per stagione, ricerca
  episodi mancanti, calendario, ricerca manuale.
- **Torrent** — libtorrent embedded: coda, limiti, tag, peer, tracker, file,
  spostamento storage, politica di seeding, fastresume, killswitch VPN e recupero
  dopo riavvio. I torrent **stalled** vengono messi realmente in pausa e fuori
  dagli slot attivi, poi riprovati automaticamente. In aggiunta: pausa, sequenziale, salta-verifica, cima-coda,
  primo/ultimo pezzo, solo-metadati; priorità per-file, web seed, modifica
  tracker, super seeding ed export `.torrent`/magnet nel dettaglio torrent.
- **Fumetti** — monitoraggio GetComics e weekly pack. Ogni fumetto aggiunto
  (weekly pack, titolo monitorato o *Download Now*) riceve il tag **`Comic`**,
  così la regola *Percorsi NAS per categoria (tag)* lo instrada nella cartella
  configurata.
- **Integrazioni** — Trakt, Simkl, Jellyfin, Plex, notifiche Telegram/e-mail/webhook.
- **UI web** — single-page responsive, tema chiaro/scuro, completamente in
  **italiano e inglese** (traduzione a runtime con import/export YAML), con log
  viewer, salute, grafici e manutenzione.
- **Visti dai feed** — ogni release vista nelle sorgenti, raggruppata per titolo,
  consultabile anche per ciò che non è monitorato.
- **Regole di sanità automatiche** — sottotitoli hardcoded e dimensioni assurde
  (soglie per risoluzione derivate da un archivio reale) vengono rifiutati;
  nessun numero da configurare. Gli scarti sono eventi ordinari e vengono
  registrati a `DEBUG`, così il log `INFO` di produzione resta pulito.
- **Regolazione acquisizione** — delay prima del download con coda di attesa e
  interruttore per-titolo "Consenti aggiornamenti".
- **Ispezione reale dei file** — i dati `ffprobe` (HDR, codec, audio, lingue)
  sono salvati per file e **usati nei confronti di upgrade**, così le decisioni
  leggono il file archiviato vero, non solo il nome. I file nuovi sono
  analizzati al completamento; un backfill incrementale schedulato copre gli
  altri. È additivo: non declassa mai un file.
- **Robustezza** — backoff progressivo delle sorgenti con lista di reset e
  housekeeping/VACUUM programmato.
- **Automazione senza confusione** — hook eventi sotto *Integrazioni* e cartelle
  osservate sotto *Configurazione*. I file copiati nelle cartelle osservate
  vengono verificati come stabili e gli errori di importazione vengono ritentati
  con backoff, senza abbandonarli dopo un numero fisso di tentativi.
- **Libreria monotona** — Rextto non scarica mai un episodio più vecchio fuori
  dai buchi riconosciuti quando possiede già episodi successivi (un vero upgrade
  di qualità passa comunque); gap-fill e azioni manuali vincono sempre.
- **Backup** — manuali o programmati (locale, FTP, cartella cloud, Telegram).
  Salvano database e configurazione, non i media né lo stato torrent.
- **Leggero per davvero** — un solo demone usa circa **150 MB di RSS** e
  **2–3 % di un core CPU** a riposo su una libreria reale con la coda libtorrent
  attiva (misurato: 9 thread, 155 MB RSS, ~2,7 % CPU su un campione di 15 s,
  ~43 s di CPU in 17 min di attività). La RAM **sale durante il download perché
  è libtorrent a "gonfiarsi"** (cache disco automatica e buffer dei pezzi) e
  torna giù a trasferimento finito; il picco resta limitato dalle impostazioni
  della cache libtorrent, quindi per l'uso normale non c'è nulla da tarare.

## Installazione

### Installa su un server Linux

L'installer ufficiale supporta Debian, Ubuntu, Fedora, openSUSE e Arch Linux.
Installa le dipendenze, compila e installa **libtorrent** dai sorgenti, poi
scarica la build continua dell'ultimo commit di `main`. Le release stabili si
possono selezionare esplicitamente; se non esiste ancora un asset precompilato,
compila automaticamente il sorgente corrente da GitHub.
Crea anche l'utente di servizio, il servizio systemd, le directory runtime e i
database vuoti al primo avvio. Non importa dati legacy.

```bash
curl -fsSL https://raw.githubusercontent.com/buzzqw/rextto/main/install.sh | bash
```

Per installare l'ultima release stabile con tag invece della build continua:

```bash
curl -fsSL https://raw.githubusercontent.com/buzzqw/rextto/main/install.sh | \
  REXTTO_CHANNEL=stable bash
```

Esegui lo stesso comando una seconda volta per cercare aggiornamenti e riavviare
Rextto con la nuova versione, oppure lascia che sia il demone installato ad
aggiornarsi (vedi *Aggiornamento di Rextto*). Database, configurazione, download,
archivi e log restano in `/var/lib/rextto`; programma e UI sono in `/opt/rextto`.

Variabili opzionali:

```bash
curl -fsSL https://raw.githubusercontent.com/buzzqw/rextto/main/install.sh | \
  REXTTO_DATA_DIR=/srv/rextto REXTTO_PORT=5000 bash
```

Il servizio si chiama `rextto.service`:

```bash
sudo systemctl status rextto.service
sudo journalctl -u rextto.service -f
```

### Aggiornamento di Rextto

Ci sono due modi supportati per aggiornare. Entrambi installano lo stesso
payload e lasciano **intatti dati e configurazione**: tutto ciò che sta in
`/var/lib/rextto` (database, log, download, percorsi archivio) sopravvive
all'aggiornamento.

| Metodo | Comando | Note |
|---|---|---|
| Installer | riesegui il comando `install.sh` qui sopra | aggiorna anche dipendenze e unità systemd |
| Demone | `sudo rexttod --update` | aggiorna solo il payload: `rexttod`, `ui/`, `lib/`, `run.sh` |

Il programma installato vive in `/opt/rextto`:

```
rexttod     il demone (rpath $ORIGIN/lib)
ui/         la UI web compilata
lib/        la libtorrent inclusa
run.sh      launcher (imposta LD_LIBRARY_PATH e REXTTO_UI_DIR)
VERSION     il marker di release mostrato da --version
```

`rexttod --update` scarica `rextto-linux-<arch>.tar.gz`, verifica il `.sha256`
pubblicato quando la release lo fornisce e prepara il nuovo payload prima di
toccare l'installazione corrente. Se il download, il checksum o l'estrazione
falliscono, l'installazione in esecuzione resta invariata; se uno swap fallisce,
i file precedenti vengono ripristinati. Il servizio viene riavviato
automaticamente quando il comando gira come root, altrimenti viene stampato il
comando `systemctl` esatto.

```bash
rexttod --version                       # versione, build number e libtorrent
sudo rexttod --update                   # ultima build continua
sudo rexttod --update --channel stable  # ultima release con tag
sudo rexttod --update --release v0.2.0  # un tag specifico
```

L'unità systemd **non** viene sovrascritta da `--update`: le personalizzazioni
locali (utente, porte, percorsi) restano. Per rigenerarla usa l'installer. Il
marker `VERSION` scritto accanto all'eseguibile è il nome della release
(`continuous`, un tag, oppure `source-main`); il numero di build numerico è
compilato nel binario e identifica la build esatta.

### Pacchetto Linux autonomo

Ogni push su `main` (e ogni tag di release) pubblica
`rextto-linux-x86_64.tar.gz` (con il relativo `.sha256`) contenente:

```
rexttod     il demone, linkato con rpath $ORIGIN/lib
ui/         la UI web compilata
lib/        la libreria libtorrent inclusa
run.sh      launcher (imposta LD_LIBRARY_PATH e REXTTO_UI_DIR)
README.md   avvio rapido e prerequisiti
```

Si estrae e si esegue senza compilatore:

```bash
mkdir rextto && tar -xzf rextto-linux-x86_64.tar.gz -C rextto
cd rextto
./run.sh --version
REXTTO_DATA_DIR="$PWD/data" REXTTO_DRY_RUN=1 REXTTO_ACTIVE=0 ./run.sh
```

Poiché l'archivio include libtorrent e il demone trova da sé la `ui/` accanto a
sé, non serve alcuna libtorrent di sistema. L'archivio è prodotto da
[`scripts/package-linux.sh`](scripts/package-linux.sh) ed è quello che
`rexttod --update` installa. Richiede un Linux 64 bit recente (glibc, libstdc++,
OpenSSL 3, zlib, libzstd); `ffprobe` è opzionale. Gli asset precompilati sono
pubblicati **solo per x86_64**: su `aarch64` l'installer ripiega sulla
compilazione da sorgente e `rexttod --update` segnala che non esiste un asset.
Per un'installazione gestita come servizio usa l'installer descritto sopra.

### Compila da un checkout (sviluppo)

Per gli sviluppatori, installa le dipendenze di compilazione e compila il demone:

```bash
cargo build --release
# binario: target/release/rexttod
```

La UI è un bundle statico servito dal demone:

```bash
rustup target add wasm32-unknown-unknown
cargo install cargo-leptos
cargo leptos --manifest-path ui/Cargo.toml build --release --frontend-only
```

Per una prova locale in modalità dry-run, senza download reali:

```bash
REXTTO_DATA_DIR="$PWD/data" REXTTO_ACTIVE=0 REXTTO_DRY_RUN=1 \
  cargo run --release -- --dry-run
```

Per installare il servizio reale usa l'installer descritto sopra.

### Verifica

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

### Leggere i log

Il log segue sempre il percorso dell'operazione, non mostra l'hash come unica
identità leggibile:

1. **CYCLE STARTED** — indica la modalità e il dominio (`full`, Serie, Film o
   Fumetti).
2. **Step 1/2** e **Step 2/2** — indicano quante sorgenti e quanti titoli vengono
   analizzati; eventuali sorgenti irraggiungibili riportano nome e motivo.
3. **Gap fill** — distingue ciò che è stato trovato nell'archivio da ciò
   che deve essere cercato online.
4. **Download started / Gap filled** — mostra titolo, episodi, sorgente e
   punteggio. Se un candidato viene saltato, il log indica motivo e decisione.
5. **CYCLE REPORT** e **CYCLE DOWNLOADS** — riassumono durata, release raccolte,
   download, upgrade, lacune ed errori.
6. Gli eventi torrent spiegano metadati ricevuti, spostamenti su NAS o dal RAM
   disk, completamento, rinomina, seeding, recovery e rimozione.

Le righe hanno formato `data ora LIVELLO [componente] messaggio · campo: valore`.
Nome o titolo sono sempre presenti nei messaggi torrent; l'hash resta soltanto un
campo tecnico per correlare un errore. `INFO` mostra il percorso normale, `WARN`/
`ERROR` spiegano cosa non è riuscito e quale risorsa è coinvolta, `DEBUG` aggiunge
i dettagli diagnostici quando è abilitato (anche gli scarti ordinari di filtro e
sanità vivono qui).

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
| **Integrazioni** | Trakt, Simkl, Jellyfin, Plex, hook eventi |
| **Manutenzione** | Backup, duplicati, scoring, riavvio |
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
| `?` | mostra l’aiuto dei tasti |
| `↑↓` | seleziona un torrent nella scheda Torrents |
| `PgUp/PgDn`, `Home/End` | naviga a pagine nei torrent e nei log |
| `Invio` | apre i dettagli del torrent selezionato (`Invio`/`Esc` per tornare) |
| `r` | aggiorna · `c` avvia un ciclo (full/serie/film/fumetti) · `q` esci |
| `s` / `e` | ricerca manuale / eventi recenti torrent |
| `a` | **aggiungi un magnet o URL `.torrent`** |
| `t` | **aggiungi un file `.torrent`** (percorso) |
| `p` / `b` | pausa/riprendi · riavvia il torrent selezionato |
| `d` | rimuovilo (chiede se eliminare anche i file) |
| `X` | rimuove i completati che hanno raggiunto i limiti di seed |
| `k` / `R` | recheck / reannounce |
| `n` / `i` / `u` | non rinominare · pin · unpin |
| `L` | imposta i limiti globali download/upload (KiB/s) |
| Dettagli: `1-4` | generale / tracker / contenuto / peer |
| `x` | nella scheda Health, pulisce il trash (chiede conferma) |
| `/` / `f` | nella scheda Logs, filtra / attiva-disattiva il follow |

### Riga di comando

Il demone parte normalmente come servizio systemd. Eseguito direttamente,
`rexttod` accetta anche queste opzioni:

| Opzione | Cosa fa |
|---|---|
| `-h`, `--help` | mostra il riepilogo d'uso |
| `-V`, `--version` | mostra versione installata, build number e libtorrent inclusa |
| `--config <file>` | usa un file di configurazione specifico (default `rextto.json`) |
| `--dry-run` | avvia senza download reali |
| `--update` | scarica e installa l'ultimo payload (vedi *Aggiornamento di Rextto*) |

Opzioni di `--update`:

| Opzione | Cosa fa |
|---|---|
| `--repo <owner/name>` | repository GitHub da cui scaricare (default `buzzqw/rextto`) |
| `--channel <name>` | `continuous` (default) o `stable` |
| `--release <tag>` | installa un tag di release specifico |
| `--install-dir <dir>` | directory di installazione (default: quella del binario) |
| `--archive <file>` | installa da un archivio locale invece di scaricare |
| `--force` | reinstalla anche se la versione è invariata (ha senso solo con `--release`: i canali rolling `continuous` e `stable` scaricano sempre l'ultimo asset) |
| `--no-restart` | non riavviare `rextto.service` dopo l'installazione |

Esempi:

```bash
rexttod --version                          # cosa è installato ora
sudo rexttod --update                      # ultima build continua
sudo rexttod --update --channel stable     # ultima release con tag
sudo rexttod --update --release v0.2.0     # un tag specifico
rexttod --update --install-dir /srv/rextto --no-restart
rexttod --update --archive ./rextto-linux-x86_64.tar.gz   # offline
```

Per provare un aggiornamento senza toccare un'installazione reale, combina
`--install-dir` con una directory usa e getta e `--no-restart`; `--archive` evita
del tutto la rete.

### Dove trovare i dettagli

Il README è la panoramica pratica; il [manuale](docs/MANUAL.it.md) documenta ogni
schermata. Indice rapido:

| Argomento | README | Manuale |
|---|---|---|
| Installazione e servizio | *Installazione* | [1. Primo avvio](docs/MANUAL.it.md#1-primo-avvio) |
| Aggiornamento, versione, pacchetto | *Aggiornamento di Rextto*, *Riga di comando* | [1. Primo avvio](docs/MANUAL.it.md#1-primo-avvio) |
| Primo avvio e modalità | *Primo avvio* | [1. Primo avvio](docs/MANUAL.it.md#1-primo-avvio) |
| Dashboard, cicli, statistiche | *Cicli e download* | [2. Dashboard](docs/MANUAL.it.md#2-dashboard) |
| Torrent, stalled, storico | *Cicli e download* | [3. Scarico](docs/MANUAL.it.md#3-scarico) |
| Serie, episodi, gap | *Aggiungi serie e film* | [4. Serie TV](docs/MANUAL.it.md#4-serie-tv) |
| Film | *Aggiungi serie e film* | [5. Film](docs/MANUAL.it.md#5-film) |
| Esplora, Archivio, Fumetti | *Le sezioni della UI* | [6. Esplora, Archivio, Fumetti](docs/MANUAL.it.md#6-esplora-archivio-fumetti) |
| Sorgenti, punteggi, rinomina | *Configura le sorgenti* | [7. Configurazione](docs/MANUAL.it.md#7-configurazione) |
| Trakt, Jellyfin, hook | *Le sezioni della UI* | [8. Integrazioni](docs/MANUAL.it.md#8-integrazioni) |
| Backup, duplicati, DB | *Le sezioni della UI* | [9. Manutenzione](docs/MANUAL.it.md#9-manutenzione) |
| Salute, log, grafici | *Leggere i log* | [10. Salute, Log, Grafici](docs/MANUAL.it.md#10-salute-log-grafici) |
| Notifiche | *Le sezioni della UI* | [11. Notifiche](docs/MANUAL.it.md#11-notifiche) |
| Problemi comuni | — | [12. Risoluzione problemi](docs/MANUAL.it.md#12-risoluzione-problemi) |

### Dati e log

- Data directory di default: `data/` (modificabile con `REXTTO_DATA_DIR`).
- Log in `data/rextto.log`, con rotazione a 5 MB (file attivo + 3 backup),
  consultabili in streaming dalla UI. Il viewer permette filtro e follow/pause.

### Variabili d'ambiente

| Variabile | Scopo |
|---|---|
| `REXTTO_DATA_DIR` | Data directory (database, log, download) |
| `REXTTO_LISTEN` | Indirizzo UI/API (default `0.0.0.0:5000`) |
| `REXTTO_ENGINE_LISTEN` | Canale interno del motore (default `127.0.0.1:8889`) |
| `REXTTO_UI_DIR` | Directory della UI web compilata (installazioni pacchettizzate) |
| `REXTTO_INSTALL_DIR` | Directory di installazione usata da `--update` |
| `REXTTO_REPO` | Repository GitHub usato da `--update` (default `buzzqw/rextto`) |
| `REXTTO_ACTIVE` | `1` abilita i cicli di acquisizione |
| `REXTTO_DRY_RUN` | `1` disabilita i download reali |
| `REXTTO_API_TOKEN` | Token bearer opzionale per API/UI |
| `RUST_LOG` | Filtro tracing (default `rextto=info`) |
| `REXTTO_URL` | Client TUI: URL base del demone (default `http://127.0.0.1:5000`) |
| `REXTTO_CHANNEL` / `REXTTO_VERSION` | Installer/updater: canale (`continuous`, `stable`) o tag di release specifico |
| `REXTTO_PORT` / `REXTTO_ENGINE_PORT` | Installer: porta UI/API (default `5000`) e porta motore (default `8889`) |
| `REXTTO_USER` | Installer: utente di servizio da creare/usare (default `rextto`) |
| `REXTTO_SKIP_PACKAGES` | Installer: `1` salta l'installazione dei pacchetti di sistema |
| `REXTTO_SKIP_LIBTORRENT_BUILD` | Installer: `1` salta la compilazione di libtorrent dai sorgenti |
| `REXTTO_SOURCE_REF` | Installer (fallback): ref GitHub da compilare (default `main`) |

## Sviluppo

### Compila, prova ed esegui

```bash
cargo build --profile fast       # build rapida del demone (target/fast/rexttod)
cargo build --release            # build del demone per produzione
cargo check                      # feedback più rapido
cargo test --all-targets         # test
cargo clean                      # rimuove gli artefatti quando serve
```

La UI web è un workspace Leptos/WASM separato:

```bash
rustup target add wasm32-unknown-unknown
cargo install cargo-leptos
cargo leptos --manifest-path ui/Cargo.toml build --release --frontend-only
```

`scripts/acceptance.sh` esegue il collaudo isolato descritto in
[`docs/ACCEPTANCE.md`](docs/ACCEPTANCE.md): usa una data directory temporanea e
porte dedicate in dry-run, quindi non tocca mai un'installazione reale.

### Packaging

[`scripts/package-linux.sh`](scripts/package-linux.sh) produce l'archivio
autonomo usato dall'installer e da `rexttod --update`:

```bash
cargo build --release --locked
cargo leptos --manifest-path ui/Cargo.toml build --release --frontend-only
scripts/package-linux.sh --binary target/release/rexttod --ui ui/target/site
# -> rextto-linux-x86_64.tar.gz + rextto-linux-x86_64.tar.gz.sha256
```

L'archivio contiene `rexttod`, la `ui/` compilata, la libtorrent in `lib/`, il
launcher `run.sh` e un breve README. Il demone è linkato con rpath `$ORIGIN/lib`
e cerca una `ui/` accanto a sé, quindi parte direttamente dall'archivio
estratto; `install.sh` copia lo stesso payload in `/opt/rextto`.

Per provare l'updater in locale senza sostituire il binario del checkout,
puntalo a una directory usa e getta usando l'archivio appena creato:

```bash
scripts/package-linux.sh --output /tmp/rextto-linux-x86_64.tar.gz
mkdir -p /tmp/rextto-install
target/release/rexttod --update --archive /tmp/rextto-linux-x86_64.tar.gz \
  --install-dir /tmp/rextto-install --no-restart
/tmp/rextto-install/rexttod --version
```

### Come sono fatte le parti

| Componente / risorsa | Ruolo | Funzioni che abilita |
|---|---|---|
| Rust + Tokio + Axum | runtime del demone e server HTTP | cicli, API REST, stream SSE dei log, UI statica |
| Leptos + WASM (`ui/`) | front-end single-page | dashboard, schermate libreria, impostazioni, UI bilingue |
| SQLite (rusqlite bundled) | persistenza locale | serie/episodi, film, archivio, fumetti, config, statistiche cicli, metadati torrent |
| libtorrent (`native/libtorrent_bridge.cpp`, `src/libtorrent.rs`) | motore BitTorrent integrato | coda e limiti, politica di seeding, tracker, file, peer, fastresume, killswitch VPN |
| reqwest + scraper + quick-xml (`src/rss.rs`, `src/websearch.rs`) | acquisizione sorgenti | feed RSS/HTML, indexer Torznab, motori web, fallback FlareSolverr |
| TMDB / TVDB (`src/tmdb.rs`, `src/tvdb.rs`) | provider metadati | locandine, stagioni, date episodi, scoperta |
| ffprobe / MediaInfo (`src/mediainfo.rs`) | ispezione reale dei file | dati codec/HDR/audio/lingue usati nei confronti di upgrade |
| Trakt / Simkl / Jellyfin / Plex (`src/integrations.rs`) | integrazioni media server | watchlist, calendario, scrobble, refresh libreria |
| Telegram / SMTP / webhook (`src/notifier.rs`) | notifiche | avvisi di completamento/errore, webhook firmati HMAC |
| FTP / cloud / Telegram (`src/backup.rs`, suppaftp) | backup programmati | snapshot di database e configurazione |
| zip, flate2, sha1/hmac/sha2 | utilità | gestione archivi, hashing, firma webhook |
| parser + rules + scoring (`src/parser.rs`, `src/rules.rs`, `src/config.rs`) | logica di dominio | parsing release, punteggio qualità, controlli di sanità, upgrade |
| CLI + self-update (`src/cli.rs`, `src/update.rs`) | operazioni | `--version`/`--help`, download release con checksum, swap atomico del payload e rollback |
| installer + packaging + systemd | operazioni | install da sorgente/release, unità di servizio, archivio autonomo |
| importatore legacy (`src/importer.rs`) | CLI di migrazione | import una tantum dei database di un'installazione precedente |

Le build di sviluppo restano piccole e non crescono all'infinito:

- I profili `dev`/`test` usano solo tabelle di riga e nessuna info di debug per le
  dipendenze (`Cargo.toml`): `target/debug` resta intorno a 1 GB invece di ~10 GB.
- Gli script di supporto per lo sviluppo sono intenzionalmente locali e ignorati
  da Git; non servono all'installer né a un'installazione di produzione.

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
