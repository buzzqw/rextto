# Rextto — Ricerca funzionalità da Sonarr, Radarr, qBittorrent, BiglyBT e autobrr

Data: 2026-09-25 · Stato: ricerca + integrazione (regole built-in, hook eventi,
cartelle osservate, smart episode, delay, upgrade per titolo, media info,
AddOptions, provider backoff, housekeeping)

Questo documento nasce da un'analisi approfondita dei sorgenti di cinque
progetti maturi, con l'obiettivo di trovare funzionalità utili da portare in
Rextto. Il codice è stato letto direttamente (non solo la documentazione):
`src/NzbDrone.Core` di Sonarr e Radarr (C#/.NET), `src/` di qBittorrent
(C++/Qt + libtorrent), `core/src/com/biglybt/` di BiglyBT (Java), `internal/`
di autobrr (Go).

Rextto è un daemon monolitico Rust (Axum + SQLite + libtorrent embedded + UI
Leptos/WASM). Ha già: sorgenti RSS/HTML/Torznab/web search con fallback
FlareSolverr, scoring qualità, upgrade automatici, metadati TMDB, monitoraggio
stagioni, ricerca mancanti, calendario, ricerca manuale, GetComics,
integrazioni Trakt/Simkl/Jellyfin/Plex/Telegram/email/webhook, libtorrent
(coda, limiti, tag, peer, tracker, file, storage move, seed policy, fastresume,
killswitch VPN), blocklist, backup, "visti nei feed", storico cicli, salute,
log live, i18n IT/EN.

---

## 1. Sintesi esecutiva

La cosa più interessante emersa è che **cinque progetti indipendenti convergono
sullo stesso primitivo**: un motore di regole/predicati dichiarativo applicato
alle release, con esito motivato (accetta / rifiuta / assegna punteggio).
Cambiano i nomi — Sonarr/Radarr parlano di *custom formats* e *release
profiles*, qBittorrent di *RSS download rules*, autobrr di *filter engine*,
BiglyBT di *subscription filter* e *tag constraints* — ma la sostanza è la
stessa. Rextto aveva solo una soglia di punteggio additiva e filtri cablati.

La seconda convergenza è **l'estensibilità tramite comandi esterni**: "esegui
questo programma a questo evento" esiste in tutti e cinque (qBittorrent
`runExternalProgram`, Sonarr `CustomScript`, BiglyBT `exec-on-assign`, autobrr
`exec action`).

Terza convergenza: **ingestione da cartelle osservate** (qBittorrent
*watched folders*, BiglyBT `TorrentFolderWatcher`).

### Priorità per valore/sforzo

| # | Funzionalità | Fonte principale | Valore | Sforzo | Stato |
|---|---|---|---|---|---|
| 1 | Motore regole release + custom formats + rejection reasons | Sonarr/Radarr/autobrr/BiglyBT/qBittorrent | alto | M | **scartato**: troppo complesso per l'utente; salvato come regole built-in (sottotitoli hardcoded + dimensione) con rejection reasons nel log |
| 2 | Inviluppi di dimensione per qualità (min/max MB) | Sonarr/Radarr | alto | S | **scartato come config**: sostituito da soglie automatiche per risoluzione derivate dall'archivio |
| 3 | Hook eventi (programma esterno) con variabili | qBittorrent/Sonarr/BiglyBT/autobrr | alto | S | **integrato** |
| 4 | Cartelle osservate | qBittorrent/BiglyBT | medio-alto | S | **integrato** |
| 5 | Smart episode guard (monotonia episodio) | autobrr | alto | S | **integrato** (core, sempre attivo) |
| 6 | Delay profile + coda "pending" | Sonarr/Radarr | alto | M | **integrato** |
| 7 | Quality profile ordinati con cutoff/gruppi | Sonarr/Radarr | alto | L | **scartato**: ridondante col range qualità; sostituito dal toggle per-titolo "Consenti aggiornamenti" |
| 8 | MediaInfo/ffprobe sul file reale | Sonarr/Radarr | alto | M | **integrato** |
| 9 | Import list (Trakt/Simkl/Plex/JSON) con esclusioni e pulizia libreria | Sonarr/Radarr | medio-alto | L | proposto |
| 10 | Duplicate profile con hash normalizzato | autobrr | alto | S-M | proposto |
| 11 | Disponibilità minima film / date di uscita | Radarr | alto | S-M | proposto |
| 12 | Provider backoff (indexer/client) | Sonarr | medio | S | **integrato** |
| 13 | Housekeeping + VACUUM pianificato | Sonarr | medio | S | **integrato** |
| 14 | IRC announce (bassa latenza) | autobrr | alto | L | analizzato |
| 15 | Remote path mapping | Sonarr/Radarr | medio | S | proposto |
| 16 | Scene mapping / XEM (anime) | Sonarr | medio | M | proposto |
| 17 | Auto-tagging per specifica | Sonarr/Radarr/BiglyBT | medio | S-M | proposto |
| 18 | Quota libreria + stub archive | BiglyBT | medio | M | proposto |
| 19 | Scheduler sorgenti con backoff | BiglyBT/autobrr | medio | S | proposto |
| 20 | Sottotitoli/sidecar + recycle bin | Sonarr/Radarr | medio | M | proposto |

---

## 2. Sonarr

### Architettura interessante
Sonarr costruisce ogni policy come piccola classe
`IDownloadDecisionEngineSpecification` scoperta via DI, ordinate per priorità e
valutate in pipeline. `DownloadDecisionMaker.GetDecisionForReport`
(`DecisionEngine/DownloadDecisionMaker.cs`) si ferma al primo gruppo di priorità
che produce un rifiuto. `DownloadRejectionReason` è un enum di ~79 valori: ogni
rifiuto è tipizzato e mostrabile in UI. I risultati sono poi ordinati da
`DownloadDecisionComparer` (qualità → custom format score → protocollo → flag
stagione intera → numero episodi → seeder → età usenet → dimensione).

### Funzionalità di rilievo
- **Custom formats** (`CustomFormats/CustomFormat.cs`,
  `CustomFormatCalculationService.cs`): insiemi di specifiche tipizzate
  (regex sul titolo, gruppo, risoluzione, sorgente, tipo, lingua, dimensione),
  raggruppate per tipo; tutti i gruppi devono combaciare, dentro un gruppo
  vince un match o falliscono le `required`. Punteggio per profilo,
  `MinFormatScore`, `CutoffFormatScore`, `MinUpgradeFormatScore`.
- **Quality definitions con inviluppo dimensione**
  (`Qualities/QualityDefinition.cs`, `AcceptableSizeSpecification.cs`):
  min/max/preferred MB per minuto, scalati sulla durata.
- **Release profiles** (`Profiles/Releases/ReleaseProfile.cs`,
  `TermMatcherService.cs`): terminiRequired / ignored, regex in stile
  `/pattern/flags`, scope per tag e indexer.
- **Delay profiles + pending releases**
  (`Profiles/Delay/DelayProfile.cs`, `Download/Pending/PendingReleaseService.cs`):
  attesa configurabile prima del grab, bypass se qualità massima o score
  custom format alto; le release temporaneamente rifiutate sono persistite e
  rivalutate.
- **MediaInfo via ffprobe** (`MediaFiles/MediaInfo/VideoFileInfoReader.cs`):
  HDR reale (Dolby Vision da `DOVIConfigurationRecord`, HDR10/HLG da primari e
  transfer BT.2020), bit depth, audio, sottotitoli.
- **Remote path mapping**, **manual/bulk import**, **import lists**,
  **auto-tagging**, **provider backoff** (`EscalationBackOff`:
  0,60,300,900,...,86400 s), **health check event-driven**, **housekeeping con
  VACUUM**, **naming token engine**.

### Cosa manca a Rextto (a parte quanto integrato)
Quality profile ordinati, delay profile, media info reale, import list,
remote path mapping, scene mapping/XEM, housekeeping.

---

## 3. Radarr

### Differenze rispetto a Sonarr
Radarr aggiunge il mondo film: identità condivisa (`MovieMetadata`) separata
dalle versioni (`Movie`, una per taglio/qualità), **disponibilità minima**
(`MovieStatusType`: Announced/InCinemas/Released/TBA, date digital/physical),
**collections** TMDB monitorate, **import exclusions** per id esterno,
**multi-version** (4K + 1080p), **edizioni** (Director's Cut, Extended,
Unrated, IMAX), **hardcoded subs detection**, **original language**.

### Punti chiave per Rextto
- **Disponibilità minima**: `Movie.cs:IsAvailable`/`GetReleaseDate` +
  `AvailabilitySpecification`. Evita di cercare/scaricare film non ancora
  usciti e abilita l'avvio automatico quando escono. Rextto oggi ha solo
  `year`.
- **Titoli alternativi/roman numerals** (`MovieTitleNormalizer`,
  `RomanNumeralParser`): match robusto di AKA e numeri romani.
- **Quality profiles ordinati** con cutoff e gruppi (`QualityProfile.cs`,
  `QualityCutoffService.cs`, `UpgradableSpecification.cs`): Rextto usa una
  singola stringa di risoluzione.
- **Custom formats** con spec Edition/Year/Size/QualityModifier e
  `MinUpgradeFormatScore`.
- **Repack/Proper revision** con confronto di release group e limite di età.

---

## 4. qBittorrent

### Architettura
`AddTorrentParams` è un oggetto ricco di opzioni passato per tutto il pipeline
di add (`sessionimpl.cpp`), e `LoadTorrentParams` persiste categoria, tag,
path, limiti e coda. `CategoryOptions` supporta **gerarchie** (`a/b/c`) con
ereditarietà campo per campo (sentinelle `DEFAULT`), `ShareLimits` con
modalità MatchAny/MatchAll e azioni Stop/Remove/RemoveWithContent/SuperSeeding.

### Funzionalità di rilievo
- **RSS auto-download rules** (`rss_autodownloadrule.cpp`): must contain /
  must not contain (wildcard→regex), filtro episodi (`NxM`, range, stagioni
  infinite), **smart episode filter** con memoria `previouslyMatchedEpisodes` e
  REPACK/PROPER, `ignoreDays`, priorità, path/categoria per regola.
- **Cartelle osservate** (`torrentfileswatcher.cpp`): polling 10 s + watcher
  FS, ricorsione→sottocategorie, retry limitati.
- **Run external program** su added/finished (`application.cpp:runExternalProgram`):
  espansione `%N %F %D %G %I %J %K %L %R %Z %T %C`, nessuna shell.
- **Per-file priorities**, **first/last piece priority**, **web seeds**,
  **tracker management**, **ban IP dinamico**, **super seeding**, **torrent
  creation**, **contenuto layout su add**, **resume data in SQLite**.

### Cosa ha ispirato l'integrazione
Il motore regole e le cartelle osservate. Il refactor `AddOptions` (opzioni
ricche su add + bridge C++ `rextto_lt_add_ex`) è la chiave per abilitare
priorità file, layout, stop condition metadata-only e web seed: consigliato
come prossimo lavoro infrastrutturale.

---

## 5. BiglyBT

### Architettura
BiglyBT modella il **tag come primitivo di automazione**: ogni tag può avere
un'espressione di classificazione (`TagPropertyConstraintHandler`, 142 KB),
azioni eseguite all'assegnazione (`TagFeatureExecOnAssign`), policy di
posizione file (`TagFeatureFileLocation`), quota massima con strategia di
rimozione/archivio stub, limiti di velocità e notifiche.

### Funzionalità di rilievo
- **Tag constraint DSL**: espressioni con `&& || == >=`, funzioni
  (`hastag, iscomplete, isshare, matches, contains, tagage, counttag,
  ifthenelse, trackerpeers…`) e keyword (`shareratio, age, size, filenames,
  fileexts, savepath, trackers, peermaxcompletion…`), con dipendenze
  static/running/time e rivalutazione throttled. È il "classificatore
  generico" che Rextto non ha.
- **Exec-on-assign actions**: bitmask `DESTROY, START, STOP, FORCE_START,
  SCRIPT, PAUSE, RESUME, MOVE_INIT_SAVE_LOC, ASSIGN_TAGS, REMOVE_TAGS,
  HOST, PUBLISH, QUEUE, BAN`.
- **Auto-tagging per estensione** (`TorrentOpenOptions.applyAutoTagging`):
  aggrega i byte per estensione, applica regole `ext → tag`, con `best_size` o
  tag di default.
- **Subscription result filter** (`SubscriptionResultFilterImpl`): with/without
  words, tag/categoria, min/max size, min/max seeder, min peer, età, regex.
- **Dedup a doppia chiave** (SHA1 infohash o name:size + UID) e stato
  libreria/archivio/storico con "mark read" globale.
- **Quota tag + stub archive** (`TagDownloadWithState.checkMaximumTaggables`,
  `DownloadStubImpl`): limite per tag con strategia archive/remove/delete.
- **Scheduler subscription** con backoff esponenziale (10 min → 8 h).
- **Global completion hooks** e spegnimento programmato.

Il punto chiave per Rextto: unificare "tag/regola" in un'unica entità con
match + azioni + posizione + limiti, consumata da un motore event-driven.

---

## 6. autobrr

### Architettura
Pipeline: annuncio (IRC o feed) → `domain.Release` → `release.Service.Process`
→ filtri ordinati per priorità → azioni → notifiche. Il filtro è un predicato
puro che accumula **rejection reasons** (`internal/domain/filter.go`,
`rejections.go`). Le definizioni indexer sono YAML (118 file) che mappano le
righe announce in release tramite regex, variabili e template.

### Funzionalità di rilievo
- **Runtime IRC** (`internal/irc/handler.go`, 2226 LOC): multi-network, state
  machine per canale, coda per canale (cap 128), SASL PLAIN + NickServ con
  escalation, retry con backoff, rilevamento flapping, proxy HTTP/SOCKS.
  È il modo per ottenere grab in secondi anziché al ciclo RSS.
- **Filter engine**: match/except (wildcard, regex, splitter che rispetta
  parentesi), resolution, codec, source, container, HDR (match doppio "DV HDR"),
  gruppo, lingua, tag, categoria, uploader, record label, seeders/leechers,
  size, origine, announce type, freeleech%, smart episode, duplicate profile,
  max downloads per finestra fissa/rolling.
- **Smart episode**: rifiuta un episodio se esiste già un download approvato di
  episodio/stagione successiva (con gestione PROPER/REPACK e daily).
- **Duplicate profile con hash normalizzato** (`release.go:Hash()`): MD5 del
  titolo normalizzato (lingua, cut, edizione, stagione, resolution, source,
  codec, HDR, audio, gruppo), più confronto per campi.
- **External filters** exec/webhook con exit/status atteso e `OnError`.
- **Action engine** ordinato con retry/replay e azioni marcate APPROVED/REJECTED.
- **Macro/template** per argomenti, webhook, path, notifiche.

---

## 7. Scelte e roadmap rimanente

Sono stati integrati (vedi §8): regole di sanità built-in, hook eventi, cartelle
osservate, smart episode, delay profile, upgrade per titolo,
MediaInfo/ffprobe, refactor `AddOptions`, provider backoff e housekeeping.

**Scartati o rimandati per scelta:**

- **Import list + esclusioni + pulizia libreria** (Sonarr/Radarr): rimandato.
  Esistono già gli import manuali Trakt/Simkl; una sincronizzazione ricorrente
  con policy e cancellazione automatica è un progetto a sé.
- **Remote path mapping** (Sonarr/Radarr): valore reale basso perché libtorrent
  è embedded e condivide il filesystem col daemon; serve solo con mount di rete
  o seedbox con path divergenti.
- **Auto-tagging** (Sonarr/Radarr/BiglyBT): rimandato finché i tag non
  pilotano delay/release profile (dipendenza).
- **IRC announce** (autobrr): progetto a sé (trasporto + parser definizioni),
  sforzo L; da valutare separatamente. Fragilità da evitare: la dipendenza dai
  template Go/sprig delle definizioni indexer.
- **Refactor `AddOptions`**: la base è integrata (paused, sequential,
  seed_mode, queue_top, first/last piece, metadata-only). Restano, da agganciare
  allo stesso bridge: priorità per-file, layout contenuto all'add, web seed,
  modifica tracker, export `.torrent`/magnet, super seeding, azioni sui limiti
  di condivisione.

**Licenze.** Sonarr/Radarr GPLv3, qBittorrent GPLv2+, BiglyBT GPLv2, autobrr
MIT; Rextto è EUPL-1.2. In questa ricerca si sono studiati *algoritmi e idee*,
non copiato codice. Eventuali port fedeli di porzioni sostanziali vanno
valutati per compatibilità (le GPL non sono compatibili con EUPL per
incorporamento diretto).

---

## 8. Integrato in questa iterazione

### 8.1 Regole di sanità built-in (`src/rules.rs`)

L'editor di regole release, i custom format e gli inviluppi di dimensione sono
stati **rimossi**: duplicavano filtri già presenti (blacklist, filtri
contenuto/sorgente, exclude per titolo) e chiedevano all'utente di indovinare
numeri. Al loro posto, controlli automatici sempre attivi:

- **Sottotitoli hardcoded** (`HC`/`hardcoded`): rifiutati, default di Radarr.
- **Dimensione assurda**: due livelli, per non escludere mai un episodio sano.
  - *hard floor* globale (2160p 300 MiB, 1080p 80, 720p 60, 576p 50, 480p 40):
    scarta solo i fake evidenti prima ancora del match col titolo.
  - *sane floor* per titolo, derivata da un archivio reale (4962 file, 5% di
    estremi grandi tagliati): 2160p 800 MiB, 1080p 120, 720p 100, 576p 60,
    480p 60. Applicata **solo per abbassare**: per una serie con storico il
    limite scende a metà dell'episodio archiviato più piccolo, mai sopra il sane
    floor; per una serie **senza storico** si usa l'hard floor permissivo. Così
    un episodio piccolo ma sano non è mai rifiutato perché la serie non ha file
    grandi. Dimensione sconosciuta = nessun rifiuto.
- **Preferenza dimensione**: piccolo bonus (max 100) per un bitrate più sano
  nella stessa risoluzione; non può mai colmare un divario di qualità.

`Config::all_release_denied_reason` unifica blacklist, content/source filter e
queste regole; `Config::release_score` = punteggio + bonus sottotitoli + bonus
dimensione. Gli scarti per regola built-in sono loggati a **INFO** con titolo,
risoluzione, dimensione e motivo (`rules::log_rejection`), gli altri restano a
DEBUG. `Release` porta `size_bytes`/`seeders`/`peers`, popolati da RSS, Torznab
e Prowlarr (`0`/`-1` = sconosciuto).

### 8.2 Hook eventi (`src/hooks.rs`)

- `EventHook { name, enabled, events[], program, args, timeout_secs }`.
- Espansione `{placeholder}` di tutti i campi del payload (anche annidati
  `a.b`) e variabili d'ambiente `REXTTO_*`; esecuzione diretta senza shell,
  timeout, output catturato e loggato.
- `Notifier` carica gli hook da `settings.event_hooks`, li espone con
  `event_hooks()`/`reload_hooks()` e li dispatcha su **ogni**
  `notify_event` (download_started, torrent_completed, season_pack_completed,
  torrent_error, comic_completed, …) su task distaccato.
- UI: pannello **Hook eventi** nella pagina **Integrazioni** (non più in una
  pagina Automazione separata).

### 8.3 Cartelle osservate (`src/watcher.rs`)

- `WatchedFolder { path, enabled, recursive, delete_after }`.
- `scan_folder`, `magnet_from_file`, `consume` (rimuove o rinomina
  `.imported`) come funzioni pure testabili.
- Worker `watched_folders_worker` in `web.rs` (ogni 15 s, salta il dry-run,
  max 5 tentativi per file) registrato tra i worker di lunga durata.
- UI: pannello **Cartelle osservate** nella tab **Acquisizione** di
  **Configurazione**.

### 8.4 Smart episode (logica core, sempre attiva)

Non è un'opzione: è una regola fissa del decision engine. In
`Database::check_series_scored_inner`, un episodio **non ancora archiviato**
viene rifiutato (`smart_episode`) quando esiste un episodio o una stagione
successivi già archiviati, con queste esenzioni:

- **gap-fill**: `ApprovalContext.gap_episode` (calcolato sui gap riconosciuti)
  passa sempre, così il backfill deliberato non è mai bloccato;
- **upgrade reale**: se la risoluzione candidata è superiore a quella del
  migliore episodio successivo archiviato, l'episodio passa;
- **azioni manuali**: `check_series_manual_scored` bypassa sempre;
- i **season pack** non sono interessati (logica per-episodio già esistente).

Se l'episodio bersaglio è già archiviato, la regola non interviene affatto e
vale il normale confronto di upgrade (soggetto a `forbid_upgrade`). In pratica
la libreria resta monotona senza mai impedire di riempire un buco.

### 8.5 Delay profile

- `pending_downloads` esteso con `due_at` preciso (minuti, non solo ore) e una
  nuova tabella `pending_movies`.
- `Config::delay_minutes(kind)` (`delay_torrent_minutes` /
  `delay_movies_minutes`) e `delay_bypass_score`.
- L'orchestratore mette in attesa serie/film e bypassa per gap-fill, ricerca
  manuale e punteggio sopra soglia; il migliore visto durante l'attesa vince.

### 8.6 Upgrade per titolo (in sostituzione dei quality profile)

I quality profile sono stati **valutati e rimossi**: il campo `quality` di
Rextto esprime già gli stessi casi (`720p` = 720p+, `1080p` = 1080p+,
`720p-1080p` = intervallo) e lo scoring additivo gestisce la preferenza tra
risoluzioni. Un secondo modello sarebbe stato ridondante.

Al loro posto, l'unica capacità realmente assente è ora un toggle per titolo:

- `SeriesConfig.disable_upgrades` / `MovieConfig.disable_upgrades` (default
  `false` = aggiornamenti consentiti), esposto in UI come **"Consenti
  aggiornamenti"**, persistito per le serie nel JSON `_migrated_series` e per i
  film in `movies_config.disable_upgrades`.
- Quando disattivo, `ApprovalContext.forbid_upgrade` impedisce che una release
  migliore sostituisca un file già archiviato (`upgrades_disabled`).

### 8.7 MediaInfo/ffprobe

- `src/mediainfo.rs`: probe via `ffprobe` con parsing puro (bit depth, HDR
  incluso Dolby Vision da side data, audio, sottotitoli, durata).
- Risultato persistito in `episodes.media_info_json`/`movies.media_info_json`
  al completamento del post-processing; API `GET /api/media-info` e
  `POST /api/media-info/probe`.

### 8.8 Refactor AddOptions (libtorrent)

- `AddOptions { paused, sequential, seed_mode, queue_top, first_last,
  stop_at_metadata }` e nuovi entry point nativi `rextto_lt_add_ex` /
  `rextto_lt_add_file_ex` (bitmask), `set_torrent_sequential`, `queue_top`,
  `set_first_last`.
- Le opzioni che richiedono i metadati (first/last, metadata-only) vengono
  applicate dal worker eventi via `enforce_deferred_options`.
- `/api/torrents/add` accetta i nuovi campi.
- **Completamento**: ora ci sono anche
  - **priorità per-file** (`set_file_priorities`, `FileView.priority`,
    `POST /api/torrents/{hash}/files/priority`) con selettore nella tab
    *Contenuto*;
  - **web seed** (`add_web_seeds`, `POST /api/torrents/{hash}/web-seeds`);
  - **modifica tracker** (`set_trackers`, `POST /api/torrents/{hash}/trackers`,
    editor `tier|url` per riga);
  - **super seeding** (`set_super_seeding`,
    `POST /api/torrents/{hash}/super-seeding`);
  - **export** `.torrent` (`GET /api/torrents/{hash}/export.torrent`) e magnet
    completo con tracker (`GET /api/torrents/{hash}/magnet`);
  - layout contenuto all'add: **non implementato** (nessuna API libtorrent
    affidabile senza toccare il layout di seeding).

### 8.9 Provider backoff

- `src/backoff.rs` (scala di Sonarr) + tabella `provider_status`.
- Feed e indexer in stato di backoff vengono saltati nel fan-out; successi e
  fallimenti aggiornano il livello. API `GET/POST /api/providers/status` e
  pulsante di reset nella UI.

### 8.10 Housekeeping

- `Database::housekeeping` taglia cicli, torrent in errore, "visti nel feed",
  log gap, backup di upgrade, storico opzionale e backoff scaduti, poi compatta
  con VACUUM. Worker periodico (`housekeeping_interval_hours`) e trigger
  manuale `POST /api/maintenance/housekeeping`.

### 8.11 API e UI

- `GET/POST /api/event-hooks`
- `GET/POST /api/watched-folders`
- `GET /api/media-info`, `POST /api/media-info/probe`
- `GET/POST /api/providers/status`
- `POST /api/maintenance/housekeeping`
- Torrent: `POST /api/torrents/{hash}/files/priority`, `/web-seeds`,
  `/trackers`, `/super-seeding`; `GET /api/torrents/{hash}/export.torrent`,
  `/magnet`.
- Nessuna pagina Automazione: **Hook eventi** è in *Integrazioni*, **Cartelle
  osservate** nella tab *Acquisizione* di *Configurazione*.
- Le traduzioni inglesi delle stringhe nuove sono **incorporate** in
  `src/i18n/default_translations.yml` e unite all'avvio con `INSERT OR IGNORE`
  (mai sopra le modifiche dell'utente).

### 8.12 Test

- `rules.rs`: 4 test (soglia dura, soglia adattiva, sottotitoli hardcoded,
  bonus dimensione limitato).
- `i18n.rs`: seed delle traduzioni senza sovrascrivere.
- `hooks.rs`: 6 test (espansione, split argomenti, flatten variabili, filtro
  eventi, validazione, esecuzione reale di `/bin/echo` e `/bin/sh` con env).
- `watcher.rs`: 4 test (scan piatto/ricorsivo con rumore, cartella mancante,
  lettura magnet, consume).
- `backoff.rs`: 3 test (scala, escalation con grace, recovery).
- `mediainfo.rs`: 4 test (HDR10 10-bit con audio/sottotitoli, Dolby Vision,
  skip motion image, bit depth con suffissi di endianness).
- `database.rs`: test per housekeeping, provider backoff, delay pending,
  media info, smart episode, upgrade per titolo.
- `config.rs`: test per i range qualità legacy (720p, 720p-1080p) e per il
  default "Consenti aggiornamenti".
- `web.rs`: test endpoint (hook reload, watched folders, media info).
- Suite completa: **233 test verdi**.

---

## 9. Idee valutate e scartate (per ora)

- **Search plugin Python di qBittorrent**: Torznab/Jackett copre già
  l'ecosistema; eseguire Python arbitrario sarebbe un peggioramento di
  sicurezza.
- **Swarm merge di BiglyBT**: richiede un piece picker custom; non portabile su
  libtorrent senza uno storage unito.
- **Media server/transcode di BiglyBT**: delegato a Jellyfin/Plex, fuori scope.
- **Related content via DHT di BiglyBT**: richiede una rete dedicata.
- **IRC di autobrr nel breve termine**: sforzo L e dipendenza dai template
  Go/sprig delle definizioni; da affrontare come progetto separato con un
  formato di definizione proprio.
