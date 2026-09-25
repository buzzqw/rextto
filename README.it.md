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
> 🔬 Ricerca funzionalità (Sonarr/Radarr/qBittorrent/BiglyBT/autobrr):
> [`docs/FEATURE-RESEARCH.md`](docs/FEATURE-RESEARCH.md)

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
  spostamento storage, politica di seeding, fastresume, killswitch VPN e recupero
  dopo riavvio.
- **Fumetti** — monitoraggio GetComics e weekly pack.
- **Integrazioni** — Trakt, Simkl, Jellyfin, Plex, notifiche Telegram/e-mail/webhook.
- **UI web** — single-page responsive, tema chiaro/scuro, **italiano e inglese**,
  con log viewer, salute, grafici e manutenzione.
- **Visti dai feed** — ogni release vista nelle sorgenti, raggruppata per titolo,
  consultabile anche per ciò che non è monitorato.
- **Policy release** — regole ordinate accetta/rifiuta/punteggio con termini
  wildcard/regex, custom format con punteggio, limiti di dimensione per qualità,
  hook eventi verso programmi esterni, cartelle osservate e simulatore dal vivo,
  modificabili dalla nuova pagina *Automazione*.
- **Regolazione acquisizione** — delay profile con coda di attesa, profili
  qualità con cutoff, smart episode opzionale, ispezione reale dei file con
  `ffprobe`, backoff progressivo delle sorgenti e housekeeping/VACUUM
  programmato.
- **Backup** — manuali o programmati (locale, FTP, cartella cloud, Telegram).
  Salvano database e configurazione, non i media né lo stato torrent.

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
Rextto con la nuova versione. Database, configurazione, download, archivi e log
restano in `/var/lib/rextto`; programma e UI sono in `/opt/rextto`.

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
i dettagli diagnostici quando è abilitato.

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
| **Automazione** | Regole release, custom format, limiti dimensione, hook eventi, cartelle osservate |
| **Integrazioni** | Trakt, Simkl, Jellyfin, Plex |
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
| `REXTTO_ACTIVE` | `1` abilita i cicli di acquisizione |
| `REXTTO_DRY_RUN` | `1` disabilita i download reali |
| `REXTTO_API_TOKEN` | Token bearer opzionale per API/UI |
| `RUST_LOG` | Filtro tracing (default `rextto=info`) |
| `REXTTO_URL` | Client TUI: URL base del demone (default `http://127.0.0.1:5000`) |

## Sviluppo

```bash
cargo build --profile fast       # build rapida del demone (target/fast/rexttod)
cargo build --release            # build del demone per produzione
cargo check                      # feedback più rapido
cargo test --all-targets         # test
cargo clean                      # rimuove gli artefatti quando serve
```

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
