# Audit funzionale Rextto

Data: 2026-09-20  
Metodo: lettura integrale di `WORKPLAN.md`, verifica statica di route/API/UI,
compilazione, test isolati e interrogazione **in sola lettura** del servizio in
esecuzione. Questo documento non sostituisce il collaudo finale con percorsi e
magnet temporanei richiesto da `ACCEPTANCE.md`.

## Esito

| Stato nel piano | Voci | Esito audit |
|---|---:|---|
| `[x]` | 185 | Implementazioni presenti; copertura verificata tramite build, test, API read-only e UI isolata. Non tutte equivalgono a una prova end-to-end su NAS/torrent reali. |
| `[~]` | 12 | Confermate parziali: i limiti descritti nel piano sono ancora presenti; include questo audit in corso. |
| `[ ]` | 20 | Confermate non completate o intenzionalmente fuori scope. |

> Nota: dopo questo audit il piano è cresciuto (feedback giro 5) e diverse voci
> `[~]` sono state chiuse (ETA, grafico velocità, log SSE, integrazioni a
> tabella, lista dischi) e l'intero feedback giro 5 (sparkline ↑/↓, modale
> torrent, ordine "prossime uscite", dettaglio film ricco, layout film/serie,
> conferme, stagioni ignorate, anteprima rinomina). Conteggi correnti del piano:
> `[x]` 212, `[~]` 10, `[ ]` 6. Vedi le sezioni "Correzioni post-audit".

Il punto generale **AUDIT parità funzionale legacy** resta aperto: non può essere
chiuso finché esistono le voci aperte/parziali sotto elencate e il gate di
accettazione non è stato eseguito.

## Evidenze riproducibili

- `cargo test --all-targets`: **80/80** test Rust superati (include test di
  integrazione degli endpoint `/api/score/preview`, `/api/torrents/{hash}/no_rename`
  e `/api/i18n/active`, oltre a magnet/sanitize, lingue richieste, salute,
  ordinamento calendario, stagioni ignorate, id film, no-rename e extra settings).
- `cargo check` in `ui/`: superato.
- `cargo leptos build --release`: superato, inclusa compilazione WASM.
- `npx playwright test --reporter=line` in `ui/end2end/`: **6/6** test superati
  in un data-dir e porte isolati. Coprono dashboard, “Ultimi trovati”,
  navigazione principale, configurazione/manutenzione, **Salute/Grafici/
  simulatore punteggi**, aggiunta torrent + registrazione magnet e **usabilità
  mobile** (viewport 390×844, nessun overflow di pagina).
- Le API GET usate dalla UI sono state interrogate sul servizio vivo: stato,
  salute, configurazione/libreria, torrent, archivio, fumetti, calendari,
  integrazioni, backup, i18n, log, fonti e feed status hanno risposto con JSON
  valido.
- Il test E2E è stato corretto: assumeva il wizard di adozione dati, in
  contraddizione con il requisito già completato di setup automatico.

## Stato runtime verificato

- `rextto.service`: attivo, modalità reale (`active=true`, `dry_run=false`),
  bridge libtorrent integrato e 7 torrent nella sessione (4 in download, 3 in
  coda al momento del controllo).
- `legacy.service`: inattivo.
- Database Rextto presenti e leggibili; l’API riporta 57 serie, 35 film,
  2.000 episodi e 115 metadati torrent.
- L’ultimo ciclo ha completato senza errori applicativi; i log mostrano scraping
  e gap-fill. Il contatore `candidates` è il numero di release esaminate prima
  della selezione finale, non il numero dei migliori candidati: l’etichetta UI
  va chiarita se deve rappresentare questi ultimi.
- `/api/feed/status`: 92 elementi monitorati, 75 con risultati e 578 release
  abbinate al momento della verifica.

## Problemi operativi rilevati

1. **Sorgenti** — health check: 42/44 sane. Un feed configurato risponde HTTP
   403; Prowlarr non è raggiungibile. Sono problemi di configurazione/rete o di
   accesso esterno, non dimostrati risolti dal codice. Il monitoraggio fonti va
   mantenuto aperto finché entrambe non tornano sane.
2. **Torrent senza metadati** — i log riportano reannounce ripetuti per due
   torrent privi di metadata. Serve verifica del magnet/tracker e un esito
   esplicito di timeout/fallimento, non solo il retry.
3. **Fastresume** — è presente uno storico di shutdown che non ha salvato tutti
   i dati fastresume entro il timeout. Il ripristino va provato nel test finale
   con torrent controllati prima di dichiarare completa la persistenza.
4. **Download reale già attivo** — il servizio è in modalità reale, mentre la
   transizione di Fase 3 e il gate `ACCEPTANCE.md` risultano ancora aperti. Non
   è corretto dichiarare il rilascio production-ready prima del collaudo.
5. **Deriva importazione/deployment** — `systemd/rextto.service` nel repository
   usa la sorgente diretta, ma l’unità installata usa ancora la vecchia
   `import-source`. Il codice non la ricrea più e la documentazione è stata
   allineata all’import diretto, ma l’unità installata va applicata durante una
   finestra di manutenzione; non riavviare il servizio attivo solo per questo.

## Voci parziali confermate

| Area | Stato effettivo | Blocco rimanente |
|---|---|---|
| Metadati TVDB/TMDB | TMDB e passaggio `tvdbid` serie presenti | completare copertura TVDB e refresh libreria completo |
| Log ciclo | polling/UI e rotazione presenti | in dry-run e SSE non equivalenti al comportamento richiesto |
| Tabella torrent | Peers/Ratio presenti | ETA risolto: `total_size`/`total_done` esposti dal bridge |
| Aggiungi magnet | `save_path` presente | mancano scarica subito e no-rename |
| Dettaglio torrent | copia magnet presente | manca grafico velocità |
| Film lingue | requisiti aggiuntivi disponibili | mancano tre selettori con obbligatorietà e anteprima |
| Configurazione avanzata | due soglie esposte | `default_language`/`tmdb_language` sono già supportati (`web.rs:2919-2920`): esporli; le altre chiavi vanno implementate prima della UI |
| Log | numero righe presenti | risolto: stream SSE reale + EventSource nella UI |
| Integrazioni | Jellyfin/Plex presenti | risolto: watchlist/calendari Trakt/Simkl in tabella |

## Voci aperte, raggruppate per area

- **Transizione:** gate finale con path temporanei, osservazione isolata su
  5000/8889, conferma esplicita per download reale.
- **Dashboard/Grafici/Salute:** sparkline download e CPU % aggiunte; restano
  grafico live multi-serie, lista dischi, metriche con delta, grafici
  CPU/RAM/disco/ramdisk/banda, salute approfondita con permessi/servizi/errori.
- **Torrent:** ETA risolto; restano grafico velocità nel dettaglio e opzioni
  magnet ("scarica subito"/"non rinominare").
- **Serie:** azioni bulk.
- **Film:** completamento lingue multiple.
- **Esplora/Archivio:** nessuna voce aperta in questa area.
- **Fumetti:** nessuna voce aperta in questa area.
- **Configurazione:** simulatore punteggi, preset/lista wanted/custom score,
  parametri libtorrent e RAM disk, i18n UI runtime, handler browser,
  salvataggio globale con modifiche non salvate.
- **Manutenzione:** pulizia profonda filtrabile, gestione systemd, scheduling
  backup e cloud.
- **Fuori scope:** aMule/eD2k e client torrent esterni restano esclusi dalla
  parità, come dichiarato dal progetto.

## Decisione

Il progetto compila e il percorso principale UI/API è utilizzabile, ma la
parità funzionale e il gate di accettazione sono **in corso**, non completati.
Le priorità sono: ripristinare Prowlarr/feed, chiudere gli stati torrent senza
metadata e fastresume, poi eseguire `ACCEPTANCE.md` in isolamento prima di
considerare stabile la modalità reale.

## Correzioni post-audit (2026-09-20)

Verifica indipendente dei `[x]` contro il codice, con queste correzioni:

1. **Archivio → "Dal feed" non caricava**: la UI chiamava `/api/feed-status`,
   inesistente; corretta a `/api/feed/status`. Erano i due tab a restare vuoti.
2. **Popup di conferma serie/film**: aggiunto il campo **Esclusioni (exclude)**,
   che il backend già accettava ma non era bindato nella UI.
3. **Dashboard**: aggiunto il **banner esplicativo** della modalità (verde
   attivo / ambra dry-run).
4. **"Ultimi download"**: titolo e dettaglio `SxxExx`/data separati su righe.
5. **Fumetti**: il tag umanizzato è ora **cliccabile** e apre il tag.
6. **Rimosso l'ultimo riferimento a legacy** nella UI (hint in Rinomina).
7. **Font scalabile**: tutte le `font-size` convertite in `rem`, root scalabile
   e controllo A−/Testo/A+ nell'header (85–140%, persistente).
8. Documento allineato: `default_language`/`tmdb_language` **sono già** nel
   backend; il `tvdbid` Torznab riceve in realtà il TMDB id e il path Prowlarr
   JSON ignora `external_ids`; in `scan_archive_path` `found == updated` sempre.

Secondo giro di miglioramenti (sempre 2026-09-20):

9. **CPU % di sistema** esposta da `health.rs` (delta di `/proc/stat`, con
   misurazione breve al primo campione) e mostrata in dashboard.
10. **ETA torrent**: `total_size`/`total_done` aggiunti al bridge C++ e a
    `TorrentView`; colonna **ETA** ordinabile nella tabella e Dimensione/ETA nel
    dettaglio.
11. **Sparkline dell'andamento download** in dashboard (40 campioni a 4s).
12. **Watchlist/calendario Trakt/Simkl** resi come **tabella** (titolo/dettaglio/
    data) invece che JSON grezzo.
13. Rimosso un import inutilizzato in `engine.rs`.
14. **Grafico velocità per-torrent** nel dettaglio (sparkline su 40 campioni).
15. **Stream SSE reale per i log** (`/api/logs/stream`, `async_stream` + EventSource).
16. **"Prossime uscite" ordinate per data**: `/api/calendar` restituiva le voci
    nell'ordine di configurazione delle serie; ora ordina per `air_date` (test di
    regressione aggiunto).
17. **`/api/feed/status` molto più veloce**: prima faceva fino a 240 scansioni
    complete dell'archivio (373.747 righe, ~15s); ora carica l'archivio una volta,
    fa il matching in memoria e cachea la risposta per 60s.
18. **Lista dischi** (filesystem reali) in dashboard e Salute; **azioni bulk per
    i film** (seleziona tutti, imposta lingua, elimina).

Nota operativa: `rexttod` in esecuzione è ancora il binario precedente; i punti
9-10 richiedono un riavvio del servizio per comparire, non eseguito
automaticamente perché il servizio è in modalità reale con download attivi.

Terzo giro (sessione autonoma, 2026-09-21):

16. **Simulatore punteggi** `/api/score/preview` + pannello UI (campi, scomposizione
    per categoria, punteggio base e con impostazioni, match serie/film).
17. **Salute estesa**: permessi percorsi, RAM disk, ultimi errori dal log e stato
    sorgenti/indexer su richiesta (oltre a dischi/CPU già presenti).
18. **Stat card** con sotto-contatori e delta ▲/▼ dei torrent.
19. **"Scarica subito"** nell'aggiunta magnet e **"Non rinominare"** per torrent
    (flag in `torrent_meta`, saltato al completamento).
20. **Film: 3 lingue con flag "obbligatoria" e anteprima** (JSON
    `language_requirements`).
21. **Browser handler magnet**: pagina `/magnet` + registrazione protocol handler.
22. **Impostazioni libtorrent avanzate** generiche (`libtorrent_extra_settings`,
    whitelist nel bridge) + RAM disk.
23. **Grafici live** multi-serie (CPU/RAM/disco/RAM disk/↓/↑) in Grafici.
24. **Filtri rapidi** di pulizia (Porn/Adult/Script/Sample/Trailer) e scelte rapide
    per la frequenza del backup.
25. Test: 74/74 (simulatore breakdown, permessi/errori salute, no-rename, extra
    settings, ordine calendario, stagioni ignorate, id film).

Queste voci richiedono un riavvio del servizio per il lato daemon; le modifiche
UI sono già servite da disco.

Quarto giro (2026-09-21, sessione autonoma):

26. **Selettore lingua UI** (it/en) con layer di traduzione runtime `tr()` e
    fallback italiano, applicato a navigazione e titoli.
27. **Log meno vuoto**: log periodico quando il ciclo automatico è in pausa
    (daemon non attivo/dry-run).
28. **Fix layout mobile**: `.view` come griglia `minmax(0, 1fr)` elimina un
    overflow di pagina su tutte le pagine (scoperto dal nuovo test mobile).
29. **Test e2e ampliati**: nuove coperture per Salute/Grafici/simulatore,
    Scarico + registrazione magnet, selettore lingua, banner modifiche non
    salvate e usabilità mobile senza overflow (6 test).
30. **"Salva tutte le impostazioni" + banner modifiche non salvate**: registro
    condiviso `DirtySettings` popolato dai campi con Salva; barra sticky con
    Salva tutte / Ignora.
31. **Perf i18n**: la lingua attiva è letta da `/api/i18n/active` e il dizionario
    completo è scaricato solo ai caricamenti pieni (non ogni 15s).
32. **RAM `/api/feed/status` (grave)**: l'endpoint caricava **tutto** l'archivio
    (373.747 righe) in memoria ad ogni calcolo, aggiungendo ~285 MB di RSS per
    chiamata (osservato 275 → 562 → 797 MB) e impiegando 4-5 s. Ora usa una
    **finestra recente** (`recent_entries(40_000)` con LIMIT sull'indice):
    verificato su copia dei dati reali, tempo **0,13-0,16 s** e RSS che si
    stabilizza (~150-180 MB) invece di crescere. Il worker eventi non ricrea più
    `Notifier`/client TMDB ogni 5 s ma solo al cambio di configurazione.

Restano aperti i punti non risolvibili da qui: modalità reale già attiva senza
gate, feed 403/Prowlarr irraggiungibile, torrent senza metadata, fastresume e
il collaudo `ACCEPTANCE.md` con percorsi temporanei.

Quinto giro (2026-09-21, sessione autonoma — RAM e parità RAM disk):

33. **RAM disk a 3 livelli (bug grave)**: la selezione della cartella di
    download ignorava `libtorrent_ramdisk_threshold_gb`/`_margin_gb` (leggeva
    solo `libtorrent_ramdisk_min_free_bytes`, non impostata) e il RAM disk
    veniva scelto finché non era pieno al 100%. Osservato a runtime:
    `/mnt/ramdisk` = 6,0/6,0 GiB, `free_bytes=0`. Ora la soglia e il margine
    sono applicati nell'add e all'arrivo dei metadati (spostamento su disco),
    con correzione per i byte prenotati dagli altri torrent (parità con
    `_check_ramdisk_capacity` di legacy). Tre test di regressione.
34. **Analisi memoria host**: `MemTotal` 15,4 GiB, `MemAvailable` ~0,7 GiB,
    swap 0. Cause: Prowlarr 4,0 GiB (leak, risolto riavviando il container →
    ~175 MiB), tmpfs `/mnt/ramdisk` 6,0 GiB, `/tmp` 0,49 GiB (di cui 454 MiB
    copia scratch del DB). `rexttod` ~745 MiB, di cui ~256 MiB cache libtorrent
    (`cache_size=16384` blocchi da 16 KiB). Riavviare il solo `rextto` non
    libera il tmpfs.
    **Risolto**: i 6,0 GiB del RAM disk erano **download orfani** (Fondazione
    S02/S03, BarLume S06) non più presenti in nessuna sessione torrent e
    parziali; su richiesta sono stati eliminati → `MemAvailable` da 0,7 a
    11 GiB, `/mnt/ramdisk` 0%. `rexttod` post-riavvio ~325 MiB.
35. **Esposizione configurazione**: RAM disk (abilita/soglia/margine/minimo) e
    porta min/max della sessione sono ora modificabili da
    Configurazione → Libtorrent (gruppo "RAM disk e porte").
36. Test: **84/84** (nuovi test RAM disk/soglia/margine/percorso).

Sesto giro (2026-09-21, sessione autonoma — UI e operatività):

37. **Dashboard**: testata e barra modalità su una sola riga (`flex-wrap:
    nowrap` + scroll), rimossi i pulsanti duplicati dalla testata; rimosse le
    tile Download/Upload dal pannello rete (resta il grafico).
38. **Seed infinito per torrent**: tab Limiti con checkbox esplicito, valori
    correnti precompilati, badge "seed ∞" in lista e nel dettaglio.
39. **Servizi systemd**: `GET /api/services` e azioni start/restart/stop via
    helper; pannello in Salute. Riavvio senza password verificato (polkit per
    `systemctl`, sudoers per l'helper).
40. **Avanzate**: tab con `min_free_space_gb` (guardia nel ciclo),
    `trash_retention_days` (pulizia selettiva per età), `archive_retention_days`.
41. **Fastresume `std::bad_alloc`**: il bridge chiedeva `flush_disk_cache` a
    ogni `save_resume_data` durante lo shutdown, facendo fallire il salvataggio
    con `std::bad_alloc` su sessioni grandi. Rimosso il flag, aggiunto `catch
    (...)`. Da verificare al primo riavvio dopo il deploy.
42. **Script**: `scripts/acceptance.sh` (collaudo isolato in dry-run),
    `scripts/build-fast.sh` (profilo `fast` + mold), `scripts/clean.sh`,
    `scripts/dev-reload.sh` (ciclo dev ~5-10 s).
43. **TVDB**: client v4 (`src/tvdb.rs`), endpoint `POST /api/tvdb/search` e
    `GET /api/tvdb/series/{id}`, chiave `tvdb_api_key` registrata e verificata
    a runtime ("Fondazione" → Foundation 2021, id 366972); UI "Cerca su TVDB".
44. **Backup cloud**: `backup_cloud_dir` copia lo snapshot in una cartella
    sincronizzata/mount remoto.
45. **Avanzate**: `archive_cleanup_enabled`/`archive_max_age_days`/
    `archive_keep_min` con `cleanup_older_than_keeping`.
46. **Pulizia profonda per nome/metadati** (`torrent_meta.name/title/series_name`).
47. **i18n**: helper `ctx_tr()` sul context `Data`: traduzione automatica di
    titoli Panel, Metric/StatLine, gruppi impostazioni, tab e tutte le `label`
    dei campi impostazioni, oltre a testata e azioni dashboard. **213 nuove
    traduzioni EN** (totale 1618). Restano i testi inline di corpo/modali.
48. **Sorgenti/Cloudflare**: verificato che feed, motori web, indexer Torznab e
    health usano il fallback FlareSolverr su 403/Cloudflare (ext.to risolto via
    FlareSolverr → HTTP 200). Alzato il timeout del health check a 45 s perché
    Prowlarr/FlareSolverr possono richiedere >20 s; gli esiti intermittenti di
    una singola sorgente sono normali.
49. Test: **86/86**.

Settimo giro (2026-09-21 — i18n, tooltip, TVDB id, sorgenti):

50. **i18n**: wrapping automatico di **541 nodi di testo** e ~**285 attributi**
    `title`/`placeholder`; traduzione tooltip impostazioni/punteggi e messaggi
    toast/notice; **2277 traduzioni EN** nel DB. Restano non tradotte solo le
    stringhe dinamiche `format!`.
51. **Tooltip**: completati i 10 tooltip mancanti su chiavi impostazione e gli 8
    campi/riga senza `title`.
52. **TVDB id reale**: campo `tvdb_id` in `SeriesConfig` (JSON libreria),
    `TmdbAddInput` e UI (la scelta TVDB lo salva); `engine.rs` invia a Torznab
    `tvdbid` con l'id TVDB e `tmdbid` con l'id TMDB (prima il TMDB id era
    etichettato come `tvdbid`).
53. **Sorgenti/Cloudflare**: health check timeout a 45 s (Prowlarr lento);
    fallback FlareSolverr verificato (ext.to → 200).

Ottavo giro (2026-09-21 — Avanzate e log):

54. **Chiavi Avanzate completate**: `stop_on_old_page_threshold` (pagine feed),
    `rename_verify_interval` (verifica file archiviati ogni N ore),
    `move_episodes` (sposta episodi/pack in archivio, spuri nel trash, il torrent
    esce dalla sessione), `debug_enabled` (log debug + diagnostica RAM/torrent).
55. **Log informativi ma asciutti** (confronto con `legacy.log`): riepilogo
    impostazioni sessione libtorrent, "resume data saved" allo shutdown,
    limiti di velocità loggati solo al cambio, `daemon ready` con versione
    libtorrent/attivo. Diagnostica rumorosa solo a livello `debug`.
56. **Fastresume**: salvataggio reso per-torrent nel bridge (un handle/encoding
    che fallisce non abortisce più l'intero shutdown); verificato
    "resume data saved" al riavvio.

Nono giro (2026-09-21 — tabelle, rimozione torrent, titoli episodi):

57. **Tabelle**: intestazioni numeriche torrent allineate a destra (erano a
    sinistra mentre i valori a destra). Rimossa la colonna "Cartella archivio"
    dalla tabella Serie TV (resta nel dettaglio), riga più stretta.
58. **Rimozione torrent**: il pulsante apre una **modale di conferma** con le
    opzioni (solo torrent / torrent+dati / +blocklist); l'eliminazione dei file
    richiede una seconda conferma irreversibile. Conferma aggiunta anche al
    percorso "Rimozione" nel dettaglio.
59. **Stato episodio**: quando è scaricato e presente sul NAS lo stato mostra
    solo il badge **"NAS"** (prima "downloaded" + "NAS", ridondante).
60. **Anteprima rinomina**: la modale era troppo stretta e i nomi andavano a capo;
    ora è `min(1500px, 96vw)` e la tabella ha `.rename-table` con
    `table-layout: fixed` e troncamento (tooltip col nome completo).
61. **Shutdown rapido**: dopo il salvataggio resume il processo terminava a
    volte in `stop-sigterm` per ~90 s. Ora `main` fa flush dei log e
    `std::process::exit`, quindi stop→start ~6 s.
62. **Testata dettaglio serie (stile legacy)**: nuovo endpoint
    `GET /api/series/{name}/info` (TMDB: nome, trama, poster, rete, anno,
    stagioni, voto) e header hero nel dettaglio con **poster + titolo + badge
    (anno, rete, percorso, stato) + trama** su una riga; seconda riga con le
    azioni (Torna all'elenco, Cerca mancanti, Scansiona archivio, Aggiorna da
    TMDB, Anteprima rinomina, **Modifica serie**). Il form di modifica è
    **collassato** dietro "Modifica serie", così il blocco dettagli resta a
    **due righe**. Verificato su "Alien Pianeta Terra" (poster, FX, trama 2025).
63. **Prestazioni caricamento (lentezza percepita vs legacy)**: le richieste
    erano quasi tutte <0,1 s tranne due che facevano **chiamate TMDB
    sequenziali**: `/api/calendar` (2,6 s, ~2 per serie) e
    `/api/recent-downloads` (3,1 s, ~1 per elemento). Ora eseguono le chiamate
    TMDB **in parallelo** (`JoinSet`) e i poster sono in **cache** nel client;
    aggiunta una **cache risposta** breve (calendar 120 s, recent-downloads
    30 s). Misure: calendar **2,6→0,6 s** (0,07 s in cache), recent-downloads
    **3,1→0,07 s**.
64. **Anteprima rinomina — nomi lunghi**: tolto il troncamento; i nomi vanno a
    capo su più righe (modale `min(1500px,96vw)`, `table-layout: fixed`), così
    anche i titoli molto lunghi restano leggibili.
64b. **Testata serie più ricca**: `/api/series/{name}/info` aggiunge **cast**,
    **generi**, **stato**, **numero episodi**, **voto**, **paese**,
    **ultima messa in onda**, **id/URL TVDB**. L'hero mostra trama **completa**,
    riga **Cast** (con link) e **Generi**, badge **voto**, **paese**,
    **stagioni**, **ultima onda**, **completezza (n/N · Completa/In corso)**,
    stato TMDB (**Terminata/In corso**) e attiva/in pausa.
64c. **Link TVDB**: pulsante **TVDB** che apre la serie su TheTVDB; se la serie
    non ha un id TVDB viene risolto automaticamente via ricerca TVDB. Gli
    **attori** linkano alla loro pagina persona TVDB (fallback TMDB se TVDB non
    disponibile). Verificato su "Alien Pianeta Terra" (serie TVDB 458912, cast
    con URL TVDB).
64d. **Episodi raggruppati per stagione**: le **stagioni** sono elencate una
    sotto l'altra, **collassate di default**; si espande una stagione per
    vedere le sue puntate. Ogni riga stagione mostra **posseduti/totali**
    (`total` dai conteggi TMDB `series_metadata`, fallback al numero max
    episodio). Il dettaglio serie espone `metadata` per stagione. Questo evita
    di renderizzare centinaia di puntate tutte insieme (serie lunghe come
    Grey's Anatomy).
64e. **Ricerca serie — TVDB dietro le quinte**: rimosso il pulsante "Cerca su
    TVDB" (era un doppione). Un solo pulsante "Cerca su TMDB"; se TMDB non trova
    nulla, il backend ripiega automaticamente su TVDB (`/api/tmdb/search`
    restituisce `source: "tvdb"` con poster/id TVDB). L'id TVDB continua a
    essere risolto da solo per il link serie e gli attori.
64f. **Rinomina dei file associati**: `rename_sidecars` rinomina thumbnail,
    sottotitoli (srt/sub/ass/vtt/…), nfo ecc. insieme al video; i doppioni
    (target già presente), le copie "(copia N)/(copy N)" e i **sidecar spuri**
    (stesso episodio `SxxEyy`/`NxNN` ma nome precedente) vengono spostati nel
    **trash** (o eliminati secondo `cleanup_action`). I sidecar vengono ripuliti
    **anche quando il video è già correttamente nominato** (prima venivano
    lasciati indietro). Test di regressione (87/87). Verificato su Only Murders
    (rimosso il `…01x01…-thumb.jpg` residuo, doppioni nel trash).
64g. **Controllo pattern economico (come legacy)**: `episode_name_conforms`
    verifica il prefisso `Serie - SxxEyy - ` **senza TMDB né MediaInfo**; i file
    già conformi non vengono analizzati. L'anteprima restituisce `items`
    (da rinominare) e `already_ok`; l'utente può scegliere se rinominare solo i
    primi o **anche quelli già corretti** (`force`, con conferma — come legacy).
65. **Rinomina serie — non funzionava e non loggava** (es.
    `Only.murders.in.the.building`): causa = `episode_target` usava
    `find_series_match(.., Some(season))`, che **rifiuta le stagioni non
    monitorate** (`seasons="5+"`), quindi ogni puntata fuori range veniva
    saltata e l'anteprima risultava vuota ("non me la fa rifare"). Corretto con
    `Config::find_series_by_name` (match per nome/alias senza filtri
    stagione/abilitata). Inoltre:
    - l'anteprima ora salta i file già corretti (niente no-op);
    - **log** di anteprima/esecuzione (una riga per file + riepilogo) e degli
      errori;
    - dopo la rinomina il **percorso nel DB viene aggiornato**
      (`set_episode_archive_path`), altrimenti la successiva anteprima non
      trovava più i file.
    Verificato su "Only Murders in the Building": 40/40 file con modifiche
    proposte e loggati.
    estraggono il titolo reale dal nome file (es. "Alien Earth - S01E01 -
    Neverland - [...]" → **Neverland**) invece di generare "Serie S01E01".
    Episodio pseudo `SxxE00` (marker di season pack) filtrato dalle viste, così
    non appare come primo episodio né gonfia i conteggi (verificato su
    "Alien Pianeta Terra": 8/8 episodi, titoli corretti, rinomina senza azioni).
