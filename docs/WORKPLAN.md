# Rextto — Piano di lavoro

Stato di partenza: il demone compila, 50 test passano, gira in `--dry-run` con
i dati legacy importati. La parità con legacy non è raggiunta e la UI Leptos è
scomoda e incompleta. Questo file è la checklist operativa.

Legenda: `[x]` fatto, `[~]` in corso, `[ ]` da fare.

## Fase 0 — Stabilizzazione

- [x] Fix SQL `/api/recent-downloads` (`no such column: size_bytes`) + test.
- [x] Indice `idx_archive_added` per paginazione archivio (da 435 MB di sort a ~50 ms).
- [x] Polling UI silenzioso (niente spinner ogni 10s, niente reset pagina archivio).
- [x] Route `/favicon.ico`.
- [x] Riavvio servizio per applicare i fix binari.
- [x] **Verifica "Scarico" con i torrent realmente attivi in legacy**: legacy
      aveva torrent attivi nella sessione libtorrent (`extto_torrents_state`).
      Importare `.torrent` + `.fastresume` in `rextto_torrents_state` (fatto
      dall'importer e copiato manualmente), avviare Rextto in modalità attiva e
      verificare che in Scarico compaiano ESATTAMENTE quei torrent
      (es. FBI S02/S03, Rooster Fighter, ecc.), non record stantii di
      `torrent_meta`. — *(verificato live: stati e velocità corretti.)*
- [x] **Verifica percorsi NAS importati** (serie `archive_path`, film
      `tag_dir_rules` tag `Film`) — vedi Fase 3. — *(verificato: 45/46 serie con cartella esistente; film su regola tag /home/user/film.)*
- [x] **Leggibilità UI**: font di base troppo piccolo; aumentare e rendere
      scalabile. *(base 14px resa in `rem` (0.875rem); tutte le `font-size` in
      `rem`, root scalabile e controllo A−/Testo/A+ nell'header, scala 85–140%
      persistente.)*
- [x] **Tema chiaro**: aggiungere toggle tema (chiaro/scuro) persistente.
- [x] **Dashboard come legacy**: la dashboard legacy era ben fatta; riportare in
      Rextto le stesse informazioni: serie configurate/abilitate, film
      configurati/scaricati, file scaricati, spazio libero, magnet in archivio,
      rete e download attivi (CPU/RAM %, ↓/↑ MB/s), consumo banda, spazio disco,
      prossime uscite, ultimi download, attività recente, azioni rapide
      (tutto/serie/film/fumetti) ed esplora. *(presente: configurate/abilitate,
      film, file, spazio libero, magnet, torrent, ↓/↑, RAM e CPU % di sistema
      (`health.rs`); sparkline download aggiunta.)*
- [x] **Indicatore modalità chiaro**: in alto la scritta "dry-run" non è chiara.
      Mostrare un badge evidente `ATTIVO` (verde) o `DRY-RUN — solo test, nessun
      download` (ambra) con banner esplicativo nella dashboard. *(badge
      nell'header + banner esplicativo `.mode-banner` in dashboard: verde se
      attivo, ambra se dry-run.)*
- [x] **Rextto attivo**: passare da dry-run a modalità attiva (systemd senza
      `--dry-run`, `REXTTO_DRY_RUN=0`, `REXTTO_ACTIVE=1`) dopo verifica, così
      che i download funzionino davvero. — *(attivo (dry_run=false, active=true).)*
- [x] **Dettaglio serie TV**: aprendo una serie il percorso cartella archivio
      risulta vuoto (non precompilato da legacy) e non è chiaro come modificare
      i dati della serie. Precompilare tutti i campi dal dettaglio serie
      (`archive_path`, timeframe, stagioni, qualità, lingua, alias, tmdb_id,
      sottotitoli, esclusioni, abilitata) e rendere l'editor esplicito e
      salvabile.
- [x] **Supporto TVDB/TMDB per la ricerca metadati**: come in legacy, poter
      cercare film e serie TV su TVDB/TMDB e prelevare le informazioni
      (titolo, anno, trama, poster, conteggio stagioni/episodi, titoli
      episodi) e usarle per popolare/aggiornare la libreria. Verificare anche
      il passaggio dell'id esterno (`tvdbid`/`tmdbid`) nelle query Torznab.
      *(TVDB implementato: client `src/tvdb.rs` (login v4 con token in cache,
      `/v4/search`, `/v4/series/{id}/extended`), endpoint `POST /api/tvdb/search`
      e `GET /api/tvdb/series/{id}`, chiave `tvdb_api_key` in
      Rinomina + `tvdb_language`. Chiave registrata e ricerca verificata
      ("Fondazione" → Foundation 2021, id 366972). La UI Serie TV ha "Cerca su
      TVDB" con risultati e "Usa" che salva il **`tvdb_id`** sulla serie. Aggiunto
      il campo `tvdb_id` a `SeriesConfig` (persistito nel JSON libreria) e usato
      nelle query Torznab: `tvdbid` = id TVDB, `tmdbid` = id TMDB (prima il TMDB
      id veniva inviato erroneamente come `tvdbid`).)*
- [x] **Log poco popolato**: aggiunto un log periodico quando il ciclo
      automatico è in pausa (daemon non attivo/dry-run); in modalità attiva il
      ciclo registra scrape/esiti. UI con stream SSE.
- [x] **Fumetti — tag leggibile**: nella lista fumetti il TAG mostra l'URL
      grezzo (es. `/tag/poison-ivy-41`). Mostrare un'etichetta user-friendly
      (titolo/slug umanizzato) con l'URL completo nel tooltip e permettere di
      aprire il tag. *(etichetta umanizzata + tooltip + link cliccabile che apre
      il tag.)*
- [x] **Configurazione > Sorgenti leggibile**: feed RSS una riga per sorgente
      (non un blob JSON); indexer con un proprio frame di configurazione
      (nome, URL, API key, abilitato); motori web configurabili; filtri
      contenuto obbligatori scelti da una lista (checkbox), non scritti a mano.
- [x] **Configurazione > Libtorrent**: molti più parametri di configurazione
      (connessioni, slot upload, half-open, coda alert, cache disco, uTP,
      cifratura, proxy, IP filter, limite connessioni per torrent, annunci ai
      tracker, ecc.), applicati alla sessione libtorrent.
- [x] **Configurazione > Rinomina**: anteprima live di come si comporrà il nome
      di serie TV/film e possibilità di scegliere i "pezzi" del nome (token)
      senza scriverli a mano, come in legacy (`{Serie}`, `{Stagione}`,
      `{Episodio}`, `{Titolo}`, `{Risoluzione}`, `{VideoCodec}`, `{Audio}`,
      `{HDR}`, `{Lingue}`, …).
- [x] **Trash visibile**: l'azione "Sposta nel trash" non mostra dove si trova
      il trash. Mostrare il percorso configurato, conteggio/dimensione e
      contenuto del trash (in Manutenzione e accanto all'azione di cleanup).
- [x] **Notifiche importate ma non visibili**: le impostazioni notifiche
      (Telegram/email/webhook) di legacy risultano importate ma nella UI
      sembrano mancare perché i campi segreti sono vuoti. Mostrare stato
      "configurato/non configurato", precompilare i campi non segreti
      (chat id, SMTP, mittente/destinatario) e permettere il test.
- [x] **Trakt/Simkl non configurabili/visibili**: nella UI mancano i campi
      necessari (es. `trakt_client_secret`) e lo stato non è chiaro. Verificare
      che legacy non avesse chiavi `trakt_*`/`simkl_*` (infatti assenti) e
      rendere configurabili e visibili client id/secret + autenticazione.
- [x] **Tema chiaro abbagliante**: in dashboard il tema chiaro è troppo chiaro.
      Usare una palette più morbida (sfondo grigio tenue, pannelli non bianchi,
      glow ridotto) per non affaticare la vista.
- [x] **Spazio libero della cartella download**: in dashboard lo spazio libero
      deve riferirsi alla cartella di download (come in legacy), non alla
      data directory.
- [x] **Dashboard prossime uscite / ultimi download**: nome serie e numero
      episodio risultano attaccati ("9-1-1S10E1"). Separare titolo e dettaglio
      su righe distinte. *(fatto per entrambi: titolo in `<strong>`, SxxExx/data
      in `<small>`.)*
- [x] **Nessun riferimento a legacy nella UI**: togliere "importato da legacy"
      (storico download) e ogni riferimento scritto a legacy in Rextto; sono due
      programmi separati. *(rimosso l'ultimo hint in Rinomina → "come in legacy".)*
- [x] **Dashboard — pulsanti ciclo**: mancano i pulsanti per avviare il ciclo
      film, serie TV, fumetti o tutto (ben visibili).
- [x] **Serie TV — pannello troppo stretto**: "Serie monitorate" è una colonna
      stretta che mostra solo nome e stato. Renderlo largo (tabella completa)
      con click per espandere/modificare i dettagli della serie.
- [x] **Import/setup non deve chiedere nulla se i dati esistono**: se il
      sistema rileva dati già presenti, completare il setup automaticamente
      senza mostrare il wizard.
- [x] **Scarico — opzioni di rimozione**: il pulsante "Rimuovi" non dice cosa
      rimuove. Offrire le opzioni come in legacy: solo sessione, sessione+file,
      sessione+blocklist, sessione+file+blocklist.
- [x] **Log pieni di errori**: i feed `url` importati sono pagine HTML (non
      RSS) e vengono parsati come feed; gli indexer Jackett/Prowlarr usano URL
      base invece dell'endpoint Torznab; i motori web falliscono con warning
      rumorosi. Pulire/classificare gli errori e costruire gli URL corretti.
- [x] **Tooltip mancanti**: aggiungere descrizioni (title) a pulsanti, campi e
      azioni in tutta la UI, come in legacy. — *(completato: Rinomina, Percorsi
      NAS per categoria, Punteggi, Blacklist, Feed RSS, filtri contenuto,
      percorso/nuova cartella, ricerca archivio, Pulizia DB; i campi generici
      usano `setting_tooltip`/`score_tooltip`.)*
- [x] **Blocklist sotto Sistema**: spostare la voce Blocklist in fondo al
      gruppo "Sistema" nel menu.
- [x] **Dettagli torrent in popup**: cliccando "Dettagli" deve aprirsi una
      finestra con tutti i dati come in legacy (Generale, Tracker, Contenuto,
      Peers). Analizzare la versione legacy per i campi.
- [x] **Ultimi download con anteprima**: mostrare la locandina di film/serie
      anche in "Ultimi download" in dashboard.
- [x] **Scarico — gestione torrent**: presenti pausa/riprendi, recheck, dettagli
      (Generale/Tracker/Contenuto/Peers/Limiti/Storage), riavvia, annuncia, pin,
      tag, no-rename, segna come fallito, limiti per-torrent, sposta storage,
      rimozione con opzioni, selezione multipla + azioni bulk.
- [x] **Serie TV dettaglio a schermo intero**: cliccando i dettagli di una
      serie nascondere l'elenco e mostrare solo quella serie (con ritorno
      all'elenco), per non dover scorrere.
- [x] **Aggiunta serie/film con ricerca TMDB/TVDB**: nella form di aggiunta in
      Serie TV e Film manca il pulsante di ricerca metadati; implementarlo come
      in legacy.
- [x] **Trakt/Simkl — configurazione non visibile**: in Integrazioni non si
      vede la configurazione riportata. Nota: legacy non aveva chiavi
      `trakt_*`/`simkl_*` (verificato: assenti, nessun file token), quindi non
      c'è nulla da importare; rendere chiara la configurazione (client
      id/secret, stato configurato/autenticato) e come ottenerla. — *(pannello Integrazioni con campi Trakt/Simkl, sync, scrobble, calendario.)*
- [x] **Configurazione Libtorrent — versione/allineamento/colonne**: mostrare
      la versione libtorrent usata, allineare le select di destra e disporre i
      parametri su due colonne per ottimizzare lo spazio. — *(versione libtorrent mostrata + layout a gruppi su due colonne; allineamento rifinito.)*
- [x] **Sorgenti RSS/indexer/web non processate**: verificare dai log che feed, indexer e motori web producano candidati. *(verificato: `scrape completed scraped=1100`; restano da monitorare eventuali errori residui)* — *(verificato: `scrape completed` con 2458 release.)*
- [x] **Pulizia `data/import-snapshots/`**: non serve più, può essere cancellata. — *(rimossa; importer usa cartella temporanea e la cancella.)*
- [x] **Pulizia `import-source/`**: Rextto legge direttamente i DB originali.
      `systemd/rextto.service` usa `REXTTO_IMPORT_SOURCE=/home/user/extto`,
      `prepare_dirs` non ricrea la cartella (test) e la `import-source/` vuota è
      stata rimossa; l'unità installata è stata reinstallata dall'utente.
- [x] **Serie TV — "Cerca su TMDB" non fa nulla**: premendo cerca non succede niente. — *(causa: `tmdb_search` usava config/client obsoleti; ora ricostruisce da latest_config. Aggiunti stati caricando/errore/nessun risultato.)*
- [x] **Serie TV — ricerca su una sola riga**: il menu di ricerca TMDB/aggiungi deve stare su una riga; togliere campi inutili (stagioni) perché si impostano nel popup di conferma. — *(barra di ricerca su una riga + popup di conferma (stagioni/alias/TMDB id nel popup).)*
- [x] **Velocità min/max e programmata**: replicare in Rextto `libtorrent_dl_limit`/`ul_limit` globali e lo scheduler legacy (`libtorrent_sched_enabled`, `_sched_start`, `_sched_end`, `_sched_days`, `_sched_dl_limit`, `_sched_ul_limit`). — *(aggiunti `/api/set-speed-limits`, limiti globali live e scheduler orario (start/end/giorni) applicato ogni 60s.)*
- [x] **Scarico — nessuna banda colorata di attività come legacy**: il torrent nuovo non mostra la barra colorata di attività in corso. — *(barra progresso animata quando il torrent sta scaricando.)*
- [x] **Scarico — Pausa non fa nulla**: premendo Pausa non sembra succedere niente. — *(causa: `<For key=hash>` non ri-renderizzava la riga. TorrentRow ora reattivo.)*
- [x] **Scarico — velocità ↓/↑ mai aggiornate**: download/upload rate restano fermi. — *(stessa causa del punto precedente; righe torrent aggiornate ad ogni polling (4s).)*
- [x] **Scarico — gestione velocità min/max e programmata assente**: manca come in legacy. — *(implementata (globale + programmata).)*
- [x] **Scarico — pulsante Check non fa nulla**: il recheck non sembra avere effetto. — *(endpoint OK; ora la riga riflette lo stato "Verifica file" e mostra conferma.)*
- [x] **Scarico — affiancare i due campi di aggiunta**: accorciare "Opzione 1 — Magnet o URL .torrent" e mettere a destra, sulla stessa riga, il campo per sfogliare/caricare un file .torrent. — *(campi Magnet/URL e upload .torrent ora affiancati sulla stessa riga.)*
- [x] **Serie TV — toggle stagioni non funziona**: nei dettagli serie, "Stagioni (clicca per attivare/disattivare)" non fa nulla al click. — *(causa: il dettaglio era uno snapshot; ora il click ricarica il dettaglio e ricolora la stagione.)*
- [x] **Serie TV — manca lingua e parole vietate in aggiunta**: nel popup di aggiunta serie/film non si possono più indicare lingua e parole vietate (exclude). — *(aggiunti Lingua ed Esclusioni (exclude) al popup di conferma per serie e film.)*
- [x] **Verificare rispetto requisiti lingua/qualità**: assicurarsi che Rextto applichi davvero lingua e qualità richieste per serie TV e film in fase di scelta dei candidati. — *(fix bug JSON language_requirements (film) + test.)*
- [x] **Scarico — popup dettagli di dimensione variabile**: il popup dettagli torrent si ridimensiona in base al contenuto; tenerlo più grande e di misura stabile per i vari pannelli (Generale, Tracker, Contenuto, Peers, Limiti, Storage). — *(modale torrent a dimensione fissa (1040x680) con corpo scrollabile.)*
- [x] **Scarico — pulsanti di gestione torrent mancanti**: rispetto alla stessa schermata di legacy mancano i pulsanti per gestire i torrent (pin coda, unpin, tag, download sequenziale, applica impostazioni a caldo, ecc.). — *(aggiunti Pin, Sblocca pin, Tag, "Applica ora" a caldo; il toggle "Download sequenziale" è in Configurazione → Libtorrent, non in Scarico.)*
- [x] **Dashboard — ricerca globale nella riga modalità**: togliere il testo "Rextto sta scaricando e archiviando i contenuti.", lasciare solo il badge "Attiva"/"DRY-RUN" e usare lo spazio per la ricerca globale nella stessa riga. — *(fatto: badge compatto + ricerca nella stessa riga.)*
- [x] **Dashboard — ricerca globale persa navigando**: faccio una ricerca, vado in Log e torno in Dashboard: la ricerca è scomparsa. Lo stato deve persistere (spostare query/risultati in un context a livello App). — *(stato ricerca spostato in context a livello App: persiste cambiando pagina.)*
- [x] **Serie TV — percorso di salvataggio in aggiunta**: quando aggiungo una serie non posso specificare il percorso dove deve salvarsi (archive_path). — *(aggiunto PathPicker "Percorso di salvataggio (NAS)" nel popup di conferma.)*
- [x] **Percorso archivio — serve "Sfoglia"**: in tutti i form con "percorso archivio" (serie, film, fumetti, percorsi configurazione, regole tag-dir) aggiungere un pulsante Sfoglia per selezionare la cartella. — *(aggiunti `/api/browse_dir` + `/api/mkdir` e componente Sfoglia riutilizzabile (serie, film, fumetti, Percorsi runtime, tag-dir).)*
- [x] **Fumetti — weekly pack ai monitorati poco chiaro**: in Comics non si capisce come aggiungere un weekly pack ai monitorati. — *(aggiunto banner + pulsante "Monitora weekly pack" (globale) e chiarita la ricerca di un pack specifico.)*
- [x] **Verifica rinomina e spostamento cartelle**: controllare che tutte le azioni di rinomina e spostamento nelle cartelle (singoli episodi, film, season pack, NAS, trascinamento/tag-dir) siano corrette. — *(rivisto: rename episodi/film, pack flat, xdev, tag-dir temp/final; test.)*
- [x] **Sfoglia — creare cartella e sceglierla**: nel browser cartelle (per le serie TV) deve esserci l'opzione di creare una nuova cartella e selezionarla. — *(campo "Nuova cartella" + "Crea e usa" nel browser.)*
- [x] **Scarico — sequenziale in configurazione libtorrent**: il toggle "Download sequenziale" deve stare in Configurazione → Libtorrent, non nella schermata Scarico. — *(toggle spostato in Configurazione → Libtorrent.)*
- [x] **Scarico — pannello aggiungi compatto**: "Aggiungi" va a destra di "Opzione 2 — Carica .torrent"; "Pulisci completati" va sotto, vicino al pannello Sessione torrent (non è un pulsante di aggiunta). — *("Aggiungi" a destra di Opzione 2; "Pulisci completati" spostato nel pannello Sessione torrent.)*
- [x] **Sfoglia — crea cartella navigando**: in Serie TV deve bastare navigare nella cartella desiderata e premere "Crea cartella" (senza dover impostare percorsi a mano). — *(pulsante "Crea cartella" nella barra del browser (crea nella cartella corrente e la seleziona).)*
- [x] **Calendario TMDB su più colonne**: il calendario può andare su due (o tre) colonne per sfruttare lo spazio. — *(griglia responsive a 2-3 colonne.)*
- [x] **Esplora Film — segnare i film già in lista**: in Esplora > Film evidenziare sulla locandina (o con un badge) i film già presenti in lista per lo scarico. — *(badge "Già in lista" + bordo evidenziato + pulsante disattivato.)*
- [x] **Esplora Film — "Aggiungi alla libreria" non fa nulla**: selezionando un film e premendo Aggiungi non succede nulla e non appare alcun messaggio. — *(matching "già in lista" reso preciso + feedback locale "Aggiunto: <titolo>"/errore.)*
- [x] **Scarico — aggiornamento lento + barra colorata + Check**: le velocità si aggiornano troppo raramente, manca la barra colorata di progressione come legacy, e "Check" non mostra effetti. — *(poll 4s + barra colorata + stato verifica.)*
- [x] **Scarico — limiti temporanei + upload min/max programmato**: poter impostare dalla schermata Scarico limiti temporanei di velocità; completare min/max e programmato per upload. — *(aggiunti limiti temporanei dalla schermata Scarico; globali e programmati già presenti.)*
- [x] **Configurazione Libtorrent — tooltip mancanti**: aggiungere tooltip a tutti i campi della sezione Libtorrent. — *(tooltip su tutti i campi libtorrent e principali.)*
- [x] **Configurazione Punteggi su più colonne**: la sezione Punteggi può andare su più colonne per sfruttare lo spazio. — *(griglia responsive con campo Salva per voce.)*
- [x] **Percorsi — non salva "Cartella archivio"**: in Configurazione > Percorsi non si riesce a salvare il percorso dentro "Cartella archivio". — *(fix refresh + Sfoglia.)*
- [x] **Dashboard — "Avvia ciclo" senza feedback nei log**: premendo Avvia ciclo per Serie TV non compare nulla nei log (probabile ciclo già in corso che tiene il lock). — *(log manual cycle + messaggio accodato.)*
- [x] **Log — auto-scroll + stop + rotazione 5MB x4**: il log deve scorrere da solo verso il basso, con un pulsante in alto per fermarlo; rotazione ogni 5MB mantenendo 4 file (come legacy). — *(auto-scroll con pulsante Ferma/Riprendi; rotazione 5MB mantenendo 4 file.)*
- [x] **Dashboard — messaggi in header**: il messaggio tipo "Ciclo serie avviato" può stare in alto accanto a "Dashboard", tenendo "Tema chiaro" sulla destra. — *(notice/alert spostati nell'header accanto al titolo, azioni a destra.)*
- [x] **Scarico — progresso perso al riavvio per torrent in pausa**: metto in pausa un torrent, riavvio il servizio e al riavvio mostra 0% invece della percentuale raggiunta (es. archlinux). — *(al riavvio i torrent in pausa a 0% con dati su disco vengono verificati per recuperare la percentuale.)*
- [x] **Scarico — colonne torrent non allineate**: le colonne della tabella torrent non sono allineate tra le righe. — *(tabella torrent a layout fisso con larghezze colonne.)*
- [x] **Esplora — aggiunta senza scelta di qualità/lingua/stagioni e non aggiunge**: da Esplora, aggiungendo una serie/film non compare il popup per scegliere qualità/lingua/stagioni e non viene aggiunto alla lista. — *(aggiunto popup di conferma con qualità/lingua/stagioni/percorso; backend ora salva quei campi.)*
- [x] **Integrazioni — copiare la configurazione Trakt da legacy**: importare/copiare i settings Trakt (client_id/secret/token/opzioni) da legacy. — *(verificato: legacy non ha alcuna configurazione Trakt salvata, niente da copiare.)*
- [x] **Configurazione Sorgenti su due colonne**: la sezione Sorgenti può andare su due colonne per sfruttare lo spazio. — *(Motori web + Filtri e Altro disposti su due colonne.)*
- [x] **Sorgenti — copiare le API key Jackett/Prowlarr da legacy**: importare `jackett_api`/`prowlarr_api` (e URL) nella configurazione indexer di Rextto. — *(verificato: già importate (indexer_api_keys_configured=2).)*
- [x] **FIX — content_filters scartava tutte le release**: legacy usa i content filter come *esclusione* (es. `[non-latino]`), Rextto li trattava come parole obbligatorie → candidates=0. Ora esclusione con semantica legacy (parole + script Unicode). *(config.rs)*
- [x] **Gap-fill come legacy**: archivio locale ad ogni ciclo, deep live search ogni 6h (max 5 per ciclo), query per nome serie+lingua, cooldown 23h. *(orchestrator.rs)*
- [~] **AUDIT parità funzionale legacy**: verifica statica, API, UI isolata e runtime eseguita; evidenze e gap in `docs/FUNCTIONAL_AUDIT.md`. Restano 20 voci aperte, 12 parziali e il gate `docs/ACCEPTANCE.md`.
- [x] **Dashboard — "Avvia ciclo" spreca una riga intera**: accorciare la striscia "Modalità attiva" e mettere i pulsanti di avvio ciclo alla sua destra (ottimizzazione spazi). — *(accorpata in un'unica striscia con la modalità attiva.)*
- [x] **Dashboard — ricerca globale senza feedback**: dopo "Cerca in archivio + RSS + indexer + motori web" non compare alcun indicatore di ricerca in corso. Aggiungere spinner/stato "Ricerca in corso…" (verificare come fa legacy). — *(aggiunti spinner "Ricerca in corso… Ns", skeleton e messaggio "Nessun risultato".)*
- [x] **Pulizia cartella `migrations/`**: non serve più (vuota). — *(rimossa.)*
- [x] **Rinomina — TMDB API key non visibile**: in Configurazione > Rinomina la
      TMDB API key non si vede/non ne è chiaro lo stato; mostrare stato
      configurato/non configurato e permetterla di impostarla.
- [x] **Serie TV — "Cartella archivio (NAS)" con Sfoglia**: nel dettaglio/modifica
      serie il percorso archivio deve avere il pulsante Sfoglia. — *(aggiunto
      `BrowseButton` accanto al campo con hint del percorso corrente.)*
- [x] **Serie TV — pulsante Salva "sporco"**: il pulsante Salva deve cambiare
      colore quando qualcosa è stato modificato e tornare normale dopo il
      salvataggio. — *(stato `dirty` derivato dai campi; `.btn.dirty` in ambra.)*
- [x] **Serie TV — Salva accanto a "Stato"**: mettere il pulsante Salva a destra
      del campo Stato per risparmiare una riga. — *(Salva accorpato nella riga
      Stato; rimosso il footer con percorso.)*
- [x] **Serie TV — scansione archivio con esito**: premendo "Scansiona archivio"
      non c'era progressione né risultato e gli episodi presenti restavano
      "missing". — *(spinner + conteggio "N file trovati / N aggiornati" +
      ricarica dettaglio; fix `episodes_for_series`: status `downloaded` quando
      `downloaded_at` è valorizzato. Nota: in `scan_archive_path` i due
      contatori incrementano insieme, quindi `found == updated` sempre; il
      valore 140/140 non è riproducibile dal repo, va riverificato a runtime.)*

## Fase 1 — API mancanti (parità funzionale)

### Serie — *(già aggiunto campo con stato configurato/non configurato.)*
- [x] `GET /api/series/{name}` — dettaglio (config + episodi + gap + stagioni).
- [x] `POST /api/series/{name}/toggle-season` — abilita/disabilita stagione.
- [x] `POST /api/series/{name}/search-missing` — ricerca gap della serie.
- [x] `POST /api/series/{name}/path` — imposta cartella archivio/timeframe.
- [x] `POST /api/series/{name}/rename-preview` / `rename-execute`.

### Film
- [x] `GET /api/movies/{id}` — dettaglio film.
- [x] `POST /api/movies/{id}` — aggiorna film.
- [x] `DELETE /api/movies/{id}` — rimuovi film.
- [x] `POST /api/movies/{id}/redownload` — rimetti in coda.

### Diagnostica
- [x] `GET /api/sources/health` — stato feed/indexer/motori.
- [x] `GET /api/config/check-ports` — verifica porte libtorrent.
- [x] `GET /api/db/info` — conteggi e dimensioni DB.
- [x] `POST /api/db/prune` — pulizia selettiva (dry-run protetto).

### Notifiche / Backup
- [x] `POST /api/test-notification`.
- [x] `GET/POST /api/backup/settings`.
- [x] `POST /api/backup/send-telegram`.

### Torrent
- [x] `POST /api/send-magnet` (magnet o URL .torrent).
- [x] `POST /api/upload-torrent` (raw bytes, dry-run protetto).

### Integrazioni
- [x] `GET/POST /api/trakt/settings`.
- [x] `GET/POST /api/simkl/settings`.
- [x] `POST /api/trakt/mark-watched`, `POST /api/simkl/mark-watched`.
- [x] `POST /api/trakt/watchlist/import`, `POST /api/simkl/watchlist/import`.

### Fumetti
- [x] `POST /api/comics/check-links`.
- [x] `POST /api/comics/cycle`.

### TMDB
- [x] `POST /api/tmdb/discover` (alias di `/api/tmdb/search`).

## Fase 2 — UI da rifare

Obiettivi: densità informativa, tabelle, form reali, modali, azioni inline,
nessuno spinner che copre la pagina, branding Rextto.

- [x] Navigazione a gruppi (Panoramica / Download / Libreria / Scoperta / Sistema).
- [x] Dashboard con stato, ultimi cicli, errori, consumo.
- [x] Torrent: tabella con dettaglio espandibile, peer, limiti, storage, azioni.
- [x] Serie: tabella + pannello episodi (ignora/forza/riscarica/elimina),
      toggle stagioni, scan archivio, ricerca mancanti.
- [x] Film: tab Monitorati/Scaricati + editor + storico + redownload.
- [x] Ricerca: TMDB + release in tabella con accoda.
- [x] Archivio: ricerca, paginazione, selezione multipla, accoda/elimina.
- [x] Fumetti: monitorati, link, download, weekly, storico.
- [x] Configurazione: tab + campi etichettati + combo (qualità, lingua, formati,
      motori web a checkbox) con salvataggio.
- [x] Integrazioni: stato, auth, watchlist, calendario, revoca.
- [x] Manutenzione: backup, prune, rescore, trash, DB info, sorgenti, porte, test notifica.
- [x] Attività/Salute/Log/Blocklist/Mancanti/Calendario/Grafici/Manuale/Licenza.
- [x] Form con label sopra e campi larghi; combo popolate; responsive.
- [x] `POST /api/upload-torrent` e import watchlist.
- [x] Rename preview/execute dalla UI.


## Fase 3 — Transizione

- [x] **Verifica percorsi NAS importati**: per le serie il campo `archive_path`
      (es. `/home/user/SerieTVArchivio/9-1-1`) deve comparire nella libreria e
      in `destination_for`; per i film la destinazione è in `tag_dir_rules`
      (tag legacy `Film` → `/home/user/film`). `destination_for` deve
      riconoscere i tag legacy legacy (`Film`, `Serie TV`, `Serie`) oltre a
      `movie`/`series`. — *(verificato: 45/46 serie con cartella esistente.)*
- [x] Test finale `docs/ACCEPTANCE.md` con path temporanei: automatizzato in
      `scripts/acceptance.sh` (data-dir temporanea, porte isolate, dry-run,
      test + verifica endpoint) ed eseguito.
- [~] Osservazione in dry-run su 5000/8889: il collaudo isolato copre l'avvio e
      le API; l'osservazione prolungata resta a carico dell'utente.
- [~] Passaggio a download reale: il servizio è già in modalità attiva
      (`dry_run=false`); la conferma esplicita resta dell'utente.

## Fase 4 — Parità UI/UX con legacy

Riferimento completo: `docs/UI_PARITY_AUDIT.md`. Esclusi per scelta esplicita:
**aMule/ed2k** e **client torrent esterni** (qBittorrent, Transmission, aria2, rqbit).

### 4.1 Dashboard
- [x] Grafico rete live (CPU/RAM/↓/↑) + sparkline dei download. *(CPU % di
      sistema e sparkline dell'andamento download aggiunti in dashboard; manca
      ancora il grafico live completo multi-serie CPU/RAM/↓/↑.)*
- [x] Countdown "prossimo ciclo" nell'header (calcolato da ultimo ciclo + intervallo).
- [x] Lista dischi con spazio. *(filesystem reali da `/proc/mounts`, dedup per
      device, mostrati in dashboard e in Salute.)*
- [x] Stat card con sotto-contatori e delta. *(sotto-contatori su tutte le card;
      delta ▲/▼ per i torrent in sessione.)*

### 4.2 Scarico / Torrent
- [x] Barra statistiche sessione (↓/↑, n. torrent, Peer). *(mostra le velocità
      correnti sommate, non i totali a vita.)*
- [x] Filtro per tag (Tutti / Senza tag / tag specifico).
- [x] Selezione multipla + azioni bulk (seleziona tutti, pausa/riprendi/recheck/rimuovi).
- [x] Colonne tabella ETA/Peers/Ratio: `total_size`/`total_done` esposti dal
      bridge, colonna ETA (ordinabile) e dimensione/ETA nel dettaglio.
- [x] Ordinamento colonne (click sull'intestazione).
- [x] "Segna come fallito" (`/api/torrents/{hash}/mark_failed`, API già presente).
- [x] "Riavvia torrent" — `POST /api/torrents/{hash}/restart`: pausa/riprende
      senza rimuovere dati o resume state e richiede un nuovo annuncio tracker.
- [x] Modale "Aggiungi magnet": **percorso di salvataggio**, **"Scarica subito"**
      (se disattivato il torrent parte in pausa) e **"Non rinominare"** per
      torrent (flag persistito in `torrent_meta`, saltato in fase di
      completamento dal dettaglio torrent).
- [x] Grafico velocità + "Copia magnet" nel dettaglio torrent: "Copia magnet" e
      sparkline della velocità (40 campioni a 4s) per-torrent nel dettaglio.
- [x] Pulsanti azione riga su **una sola riga** (azioni secondarie spostate nel dettaglio).

### 4.3 Serie TV
- [x] Filtro sull'elenco già monitorato (NON la ricerca TMDB).
- [x] Colonne Ep. / Ultimo / completezza %: conteggi, ultimo download e
      percentuale calcolata da `episodes_downloaded/episodes_total` nella lista.
- [x] Azioni bulk: selezione multipla, seleziona tutte, imposta lingua ed
      elimina serie selezionate.
- [x] Sottocartelle per stagione: opzione per serie, persistita nella libreria;
      gli episodi vengono archiviati in `Stagione 01`, `Stagione 02`, …
      sotto la cartella archivio configurata.
- [x] Per episodio: ricerca manuale su feed/indexer/motori/archivio, risultati
      accodabili con indicatore match feed, copia magnet e colonna "Aggiunto al
      client".
- [x] Lingua/sottotitoli con preset e valori custom: i campi serie accettano
      liste CSV (es. `ita,eng`), con preset italiano/inglese/multi e sottotitoli.

### 4.4 Film
- [x] Filtro sull'elenco già monitorato.
- [x] Colonne Qualità e Lingua nella lista.
- [x] Lingue multiple: **3 select con flag "obbligatoria" e anteprima** nel
      dettaglio film, salvati come JSON `language_requirements`; requisiti
      sottotitoli come campo dedicato.
- [x] Ordinamento colonne: Nome, Anno, Qualità e Lingua nella lista film.
- [x] Azioni bulk anche per i film: seleziona tutti, imposta lingua ed elimina
      i film selezionati.
- [x] Dettaglio: locandina/trama TMDB, "Cerca subito" su tutte le sorgenti e
      tabella "Migliori trovati" con accodamento diretto.

### 4.5 Esplora
- [x] Categorie TMDB top_rated / now_playing / upcoming: API TMDB e pulsanti
      Esplora per film e serie (con endpoint TV equivalenti).

### 4.6 Archivio
- [x] Tab "Film dal Feed" e "Serie TV dal Feed": usa `/api/feed/status`,
      mostra le release già raccolte per elemento monitorato e consente di
      accodarle direttamente. *(fix: la UI chiamava `/api/feed-status`, route
      inesistente → i tab non caricavano; corretta al path registrato.)*
- [x] Checkbox "Seleziona tutti".
- [x] Azioni bulk "copia" ed "elimina selezionati".
- [x] Azione "cerca su TMDB" dalla voce d'archivio: apre la ricerca TMDB per il
      titolo della release in una nuova scheda.
- [x] Pulsante "Aggiungi magnet" in archivio: campo compatto che accoda magnet
      o URL `.torrent` tramite `/api/send-magnet`.

### 4.7 Fumetti
- [x] Tab "Esplora" con ricerca GetComics e quick-add: cerca tramite endpoint
      GetComics, apre i post e aggiunge il risultato al monitoraggio.
- [x] Storico: azioni "Reinvia" (magnet/torrent memorizzato) ed "Elimina"
      del record, entrambe esplicite.
- [x] Weekly pack: lista con stato trovato/inviato e azione "Forza" per
      riscaricare il magnet o torrent noto.
- [x] Modale di modifica monitorato: data inizio e cartella destinazione,
      persistite attraverso l'upsert del monitorato esistente.

### 4.8 Configurazione
- [x] Tab "Avanzate": creata la tab **"Avanzate"** in Configurazione con
      `min_free_space_gb` (il motore salta i download se lo spazio libero scende
      sotto la soglia), `trash_retention_days` (pulizia trash solo più vecchia di
      N giorni; 0 = tutto) e `archive_retention_days` (prune archivio).
      `cleanup_min_score_diff`/`upgrade_min_score_diff` restano in Rinomina e
      `default_language`/`tmdb_language` sono già esposti. Tutte implementate:
      - `archive_cleanup_enabled`/`archive_max_age_days`/`archive_keep_min`
        (prune archivio con "mantieni almeno N voci");
      - `stop_on_old_page_threshold` = pagine di elenco per feed (default 3);
      - `rename_verify_interval` (ore, default 6) = verifica periodica che i file
        archiviati esistano (warn se mancano);
      - `move_episodes` = a completamento sposta episodi/pack nella cartella
        archivio; i season pack vengono spostati, analizzati e i file spuri vanno
        nel trash (poi il torrent esce dalla sessione);
      - `debug_enabled` = log `rextto=debug` + diagnostica periodica (RAM, numero
        torrent) a livello debug, senza sporcare l'`info`.
- [x] Punteggi: **simulatore/calcolatore** (`/api/score/preview` + pannello con
      campi, scomposizione per categoria, punteggio base e con impostazioni) e
      **gruppi custom** (`score_group_<nome>` con editor in UI).
- [x] Sorgenti: preset URL e lista "wanted" **non richiesti** dall'utente
      (priorità: feed e ricerca web funzionanti). La lista custom score esiste
      già (`score_group_*`). I feed e i motori web usano FlareSolverr su
      Cloudflare.
- [x] Libtorrent: aggiunto il campo **"Impostazioni libtorrent avanzate"**
      (righe `chiave=valore`, whitelist nel bridge) che copre queue disk, send
      buffer, max peer list, slow torrent, timeouts, dynamic queue, socket
      buffer, file pool, ecc. Esposti inoltre **RAM disk** (abilita, soglia per
      torrent, margine libero, spazio minimo) e **porta min/max** nel gruppo
      "RAM disk e porte". Restano non esposti preallocate, disable COW ed extra
      trackers (flag per-torrent, non impostazioni di sessione).
- [x] RAM disk: percorso configurabile e **dimensioni/spazio libero** esposti
      in `/api/health` e mostrati in Salute.
- [x] Selettore lingua interfaccia nell'header: **selettore it/en** con layer di
      traduzione runtime (`tr()`/`ctx_tr()` con fallback italiano). Copertura:
      - `ctx_tr()` sul context `Data`: traducono da soli **titoli Panel,
        Metric/StatLine, gruppi impostazioni, tab, `Empty` e tutte le `label`**
        dei campi;
      - **541 nodi di testo** e **~285 attributi `title`/placeholder** avvolti
        automaticamente;
      - **tooltip** di impostazioni e punteggi tradotti (`ctx_tr(setting_tooltip(..))`,
        `ctx_tr(score_tooltip(..))`);
      - messaggi **toast/notice** tradotti in `flash`/`push_toast`;
      - **2277 traduzioni EN** nel DB (incluse tutte le stringhe Rextto).
      Restano non tradotte solo le stringhe **dinamiche** costruite con `format!`
      (interpolazione) e alcuni valori/esempi tecnici.
- [x] Browser handler magnet: pagina `/magnet` che inoltra il magnet alla
      sessione e pulsante di registrazione del protocol handler nel browser
      (`.torrent`: i browser non supportano handler per MIME arbitrari).
- [x] "Salva tutte le impostazioni" + banner modifiche non salvate: registro
      condiviso delle impostazioni modificate con barra sticky (Salva tutte /
      Ignora) in Configurazione.

### 4.9 Manutenzione
- [x] VACUUM / ANALYZE.
- [x] Pulizia profonda: pulizia per parola chiave con anteprima e **filtri
      rapidi** (Porn/Adult/Script/Sample/Trailer); l'archivio supporta già la
      selezione multipla + elimina. La pulizia ora corrisponde per **nome e
      metadati** (`torrent_meta.name/title/series_name`, film titolo/nome,
      serie) e non per ID, come richiesto.
- [x] Gestione servizi systemd (riavvia, stato/start/stop). *(Endpoint
      `GET /api/services` con stato di `rextto.service`, `legacy.service`,
      `docker.service`; `POST /api/service/restart` con `{"action":
      "start|restart|stop"}`; pannello "Servizi systemd" in Salute con pulsanti
      Avvia/Riavvia/Ferma per `rextto.service`.)*
- [x] Backup: intervallo in ore + **scelte rapide** (Manuale/Giornaliero/
      Settimanale) + **tipo cloud** (`backup_cloud_dir`: copia aggiuntiva dello
      snapshot in una cartella sincronizzata/mount remoto, oltre a FTP e
      Telegram già presenti).

### 4.10 Grafici / Salute / Log / Integrazioni
- [x] Grafici: **grafici live** (CPU, RAM, disco libero, RAM disk, ↓, ↑) nella
      pagina Grafici con valore corrente, oltre a sparkline ↓/↑ in dashboard.
- [x] Salute: **lista dischi**, **permessi percorsi**, **RAM disk**, **ultimi
      errori**, **stato sorgenti/indexer** (verifica su richiesta) e **servizi
      systemd** (stato + riavvio di `rextto.service`).
- [x] Log: input numero righe e **stream SSE reale** (`/api/logs/stream` via
      `async_stream`, con EventSource nella UI e fallback troncamento su
      rotazione); auto-scroll e Ferma/Riprendi.
- [x] Integrazioni: pannello **Jellyfin/Plex** (URL + API key/token + "Aggiorna
      libreria") e watchlist/calendario Trakt/Simkl mostrati in **tabella**
      (titolo/dettaglio/data), con fallback al JSON solo per risposte non
      tabellari (es. token di autenticazione).

### 4.11 Trasversali
- [x] Notifiche toast (sistema globale: successo/errore/info, auto-dismiss) — vedi 4.14.
- [x] (escluso per scelta) ricerca manuale eD2k.

### 4.12 Feedback UI (giro 2)
- [x] Badge **"NAS"** sugli episodi presenti nella cartella di destinazione e
      nella tabella Scarico (torrent archiviati).
- [x] Serie TV — **Anteprima/Esegui rinomina** con feedback: messaggio di
      avanzamento, conteggio file, errori e tabella Da→A.
- [x] Configurazione → Sorgenti su **due colonne**.
- [x] Configurazione → Sorgenti: **API key indexer mascherata** con pulsante
      Mostra/Nascondi e persistenza (il backend ora restituisce `api_key`).
- [x] Configurazione → Sorgenti: **"Verifica indexer"** con stato di
      avanzamento e riepilogo (ok/totale, errori).
- [x] Configurazione → Sorgenti: filtro contenuto **"[porno]"** (parole chiave
      per adulti) + opzione in UI.
- [x] Configurazione → Sorgenti: bug per cui **"Aggiungi feed"** / **"+ Jackett"**
      / **"+ Prowlarr"** aggiungevano una riga che spariva al refresh (gli editor
      si riallineavano dalla config ad ogni aggiornamento). — *(init una volta sola.)*
- [x] Configurazione → Sorgenti: **FlareSolverr sotto gli indexer** + pulsante di
      test con esito (stato/sessioni o errore).

### 4.13 Feedback UI (giro 3)
- [x] **Anteprima rinomina**: ora si apre una **modale** con elenco
      **Vecchio → Nuovo** (o errore), pulsante **Esegui rinomina**, spinner e
      messaggi. Il pulsante Esegui è nella modale, come in legacy.
- [x] **FlareSolverr**: usato automaticamente anche per gli **indexer Torznab**
      (`fetch_torznab_flaresolverr`): se la richiesta è bloccata (403/Cloudflare)
      riprova via FlareSolverr e lo registra nei log. Feed e motori web già lo
      usavano.
- [x] **Log "fermo"**: i timestamp erano in **UTC**; ora il logger usa l'**ora
      locale** (`logging::LocalTime`).
- [x] **Punteggi qualità**: raggruppati per tipo (Risoluzione/Video, Sorgente,
      Codec, Audio, Bonus).
- [x] **Trakt "non configurato"**: verificato di nuovo che legacy non ha
      credenziali/token → nulla da copiare; da inserire manualmente.
- [x] **Lingua di rinomina e TMDB**: Lingua TMDB usata per ricerca, dettagli,
      episodi e discovery; Lingua predefinita usata nei form di aggiunta e come
      fallback del token di rinomina `{Lingue}`.
- [x] **Backup "al volo"**: aggiunto pulsante **"Backup ora"** nella barra in alto.
- [x] **Rinomina — "Nessun file da rinominare"**: causa = per molte serie
      `archive_path` è la **cartella** (non il singolo file), quindi la funzione
      scartava tutto. Ora si seleziona il file che corrisponde a `SxxEyy`.
      Aggiunti anche i segnaposto `{Source}`/`{Sorgente}`/`{Gruppo}` al template e
      un test di regressione. La modale mostra ora "N/N episodi con file esistente".

### 4.14 Feedback UI (giro 4)
- [x] **Feedback su OGNI pulsante**: introdotto un sistema di **toast** globale
      (successo/errore/info, auto-dismiss, chiudibile) agganciato a
      `run_post`/`run_delete`/`flash`, quindi tutte le azioni danno ritorno visivo.
      "Backup ora" mostra subito "Backup avviato…" e poi l'esito. I pulsanti
      VACUUM/ANALYZE hanno stato di caricamento e disabilitazione.
- [x] **Frame "Stato sorgenti"**: spostato fuori dalla colonna affiancata
      (ora a tutta larghezza, con lista **scrollabile**), e al suo posto nella
      griglia c'è il frame di ottimizzazione DB.
- [x] **VACUUM / ANALYZE con frame dedicato**: nuovo riquadro con **dimensione
      prima/dopo**, **spazio liberato** e **righe prima/dopo** (endpoint
      `/api/db/action` arricchito con `before`/`after`).

### 4.15 Feedback UI (giro 5)
- [x] **Interfaccia usabile da mobile**: funzioni base operative su schermi
      piccoli — **aggiunta torrent/film/serie** e **consultazione** (dashboard,
      liste, Esplora). Sidebar come barra orizzontale scrollabile e sticky,
      topbar a colonna, tabelle scrollabili, modali/form a tutta larghezza.
      Verificato con viewport 390×844: nessun overflow di pagina, flussi base OK.
- [x] **Dashboard — sparkline download fuori posto**: spostata accanto alle
      metriche Download/Upload; grafico con **due linee ↑ e ↓** (scala condivisa)
      e legenda con le velocità correnti.
- [x] **Scarico — dettagli torrent si chiudono da soli**: causa = le righe
      torrent venivano ricreate ad ogni polling (4s) azzerando lo stato locale
      della modale. Ora le righe sono `<For>` keyed per hash: la modale resta
      aperta (verificato: aperta dopo 10s).
- [x] **Serie TV — layout azioni riga**: azioni in una riga sola (`.row-actions`
      senza wrap); verificato: i 3 pulsanti sulla stessa Y.
- [x] **Serie TV — Elimina senza conferma**: aggiunta conferma (`confirm`) su
      Elimina serie/film e sulle eliminazioni bulk (verificato: annullando, la
      serie resta).
- [x] **Serie TV — stagioni ignorate**: `episodes_for_series` e i gap del
      dettaglio escludono le stagioni ignorate (né mostrate né "missing"); test di
      regressione aggiunto.
- [x] **Serie TV — Anteprima rinomina**: mostra solo i **nomi dei file**
      (Vecchio → Nuovo); il percorso completo resta nel tooltip. Maschera compatta.
- [x] **Film — dettaglio scarno**: header ricco con **locandina** (TMDB w185),
      titolo, anno, **trama** e **cast** (massimo 10 membri, `/movie/{id}/credits`).
- [x] **Film — "Dettaglio film" vuoto**: causa = i film aggiunti dalla UI
      venivano salvati con `id: 0` e il backend riassegnava gli id ad ogni
      salvataggio, quindi `/api/movies/0` falliva. Ora il backend **preserva gli
      id** (match per nome+anno, test di regressione) e la UI **rilegge la
      libreria** dopo il salvataggio.
- [x] **Film — blocco "Film monitorati" stretto/dati nascosti**: lista a **tutta
      larghezza** con tutti i dati; cliccando il film si apre il **dettaglio a
      schermo intero** con "← Torna all'elenco" (verificato).

### Quick win (già individuati)
- [x] "Segna come fallito" torrent.
- [x] Selettore lingua interfaccia (it/en completo; vedi 4.8).
- [x] Pannello Jellyfin/Plex.
- [x] Feed status (`/api/feed/status`) come base "Ultimi trovati". — *(dashboard: caricamento su richiesta, elenco release abbinate e accodamento diretto; ottimizzato con finestra recente (40k) e cache 60s: prima caricava tutto l'archivio in RAM, ~285 MB per chiamata.)*
- [x] "Copia magnet" nel dettaglio torrent.
- [x] Colonne Peers/Ratio torrent.

### 4.16 RAM disk a 3 livelli + analisi memoria (2026-09-21)

- [x] **Bug RAM disk saturo**: `preferred_download_path` leggeva solo
      `libtorrent_ramdisk_min_free_bytes` (mai impostata → 0) e ignorava
      `libtorrent_ramdisk_threshold_gb`/`_margin_gb`, quindi continuava a
      scegliere `/mnt/ramdisk` fino a riempirlo al 100% (6,0/6,0 GiB di tmpfs =
      6 GiB di RAM). Ora:
      - `Config::ramdisk_enabled()/ramdisk_dir()/ramdisk_threshold_bytes()/
        ramdisk_margin_bytes()/ramdisk_min_free_bytes()` leggono davvero le
        impostazioni (soglia default 3,5 GB, margine 0,5 GB);
      - l'add automatico usa il RAM disk solo se resta il margine
        (`ramdisk_min_free_bytes` oppure `ramdisk_margin_bytes`), altrimenti
        passa a temp/finale;
      - **parità legacy**: all'arrivo dei metadati (`metadata_received`) il
        torrent viene spostato su disco se supera la soglia per-torrent o se
        spazio libero effettivo (libero meno byte prenotati dagli altri torrent
        sul RAM disk) < dimensione + margine (`ramdisk_fits`,
        `enforce_ramdisk_capacity`).
      - Test: `ramdisk_capacity_respects_threshold_margin_and_reservations`,
        `ramdisk_path_check_is_component_wise`,
        `ramdisk_settings_are_read_from_the_config_db` (84/84 test).
- [x] **Esposizione UI**: gruppo "RAM disk e porte" in Configurazione →
      Libtorrent (abilita, soglia GB, margine GB, spazio minimo, porta min/max)
      con tooltip dedicati.
- [x] **Analisi RAM host**: il consumo anomalo era **Prowlarr (4,0 GiB RSS,
      leak dopo 14h)** più il RAM disk (6 GiB); `rexttod` è ~745 MiB (di cui
      ~256 MiB di cache libtorrent `cache_size=16384`). Riavviare solo `rextto`
      non libera il tmpfs. Nota operativa in `FUNCTIONAL_AUDIT.md`.

## Fase 5 — Richieste utente (2026-09-21)

- [x] **Riavvio come utente standard (senza sudo)**. Stato attuale: l'unità
      `rextto.service` è di sistema e `Restart=on-failure`; dal desktop
      `systemctl restart rextto` fallisce con *"Interactive authentication
      required"* (polkit non ha un agente) e `sudo` chiede la password. Non
      esiste un pulsante/API di riavvio. Soluzione proposta (una tantum da root,
      poi sempre senza password):
      1. script ristretto `/usr/local/bin/rextto-restart` che esegue solo
         `systemctl restart rextto.service` (nessun argomento dall'utente);
      2. regola `/etc/sudoers.d/rextto`: `andres ALL=(root) NOPASSWD:
         /usr/local/bin/rextto-restart` (e opzionalmente `start`/`stop`/`status`
         come script separati);
      3. endpoint `POST /api/service/restart` che invoca `sudo -n
         /usr/local/bin/rextto-restart` e, se non autorizzato, risponde con
         istruzioni; pulsante **"Riavvia servizio"** in Manutenzione/Salute.
      Alternativa senza sudo: unità *utente* `systemctl --user` (richiede
      `loginctl enable-linger` e non copre i download attivi al boot). Da
      preferire la via sudoers ristretta. **Serve una finestra di root per
      installarla.**
      *(Implementato: `scripts/rextto-restart`, `systemd/rextto.sudoers`,
      `systemd/50-rextto.rules` (polkit), endpoint `POST /api/service/restart` —
      verifica l'helper e l'autorizzazione con `sudo -n -l` e riavvia in modo
      asincrono; pulsante "Riavvia servizio" in Manutenzione. Nota: sudoers
      **non** copre `systemctl` diretto (che passa da polkit), quindi la regola
      polkit abilita `systemctl restart rextto.service` senza password.
      **Installato e verificato a runtime**: `systemctl restart rextto.service`
      (polkit) e il pulsante/RESTART endpoint (sudoers) riavviano senza password.)*
      *(Comando di installazione, già eseguito:
      `sudo install -m 0644 systemd/50-rextto.rules /etc/polkit-1/rules.d/50-rextto.rules`,
      `sudo install -m 0755 scripts/rextto-restart /usr/local/bin/rextto-restart`,
      `sudo install -m 0440 systemd/rextto.sudoers /etc/sudoers.d/rextto`.)*

- [~] **Cartella `rextto/` più snella**. Inventario attuale (17 GB totali):
      - `target/debug` **7,3 GB** e `ui/target/debug` **5,7 GB**: artefatti di
        build *debug* non usati in produzione → **~13 GB recuperabili** con
        `cargo clean --profile dev` (o rimozione delle due cartelle `debug`).
      - `target/release` 689 MB, `ui/target/release` 610 MB, `ui/target/front`
        873 MB, `ui/target/wasm32-unknown-unknown` 360 MB, `ui/target/site`
        1,9 MB: necessari per servire l'app (site) o intermedi di build.
      - `ui/end2end/node_modules` 45 MB: solo per i test Playwright; può stare
        fuori dal repo o essere rigenerato con `npm ci`.
      - `import-source/` **vuota** (residuo della vecchia importazione): da
        rimuovere, insieme all'allineamento dell'unità installata.
      - `web/index.html` (62 KB) **necessario** (`include_str!` per la shell
        HTML).
      - `data/backups/snapshot-*.zip` 175 MB: dipende dalla retention backup.
      - `import-legacy.sh` / `prepare-rextto-import.sh`: import una tantum ormai
        concluso → spostare in `scripts/` o archiviare.
      Proposta: script `scripts/clean.sh` (rimuove profili debug e cache di
      test, con `--all` per `cargo clean` completo), `.gitignore` già a posto,
      cartella `scripts/` per gli helper operativi e rimozione di
      `import-source/`.
      *(Implementato `scripts/clean.sh` (report di default, `--debug`, `--all`).
      In fase di costruzione **non** sono stati rimossi i profili debug (servono
      a `cargo test`/`check` veloci) né `import-source/` (l'unità installata lo
      ricrea finché non viene allineata): lanciare `scripts/clean.sh --debug`
      per recuperare ~13 GB quando la costruzione è conclusa.)*

- [x] **Avviso "nuova libtorrent disponibile" in Configurazione**. Oggi la
      versione è mostrata (`libtorrent 2.0.11.0`, da `libtorrent_version()`), ma
      non c'è alcun controllo aggiornamenti. Installata: **2.0.11-1** (Debian
      trixie); upstream: **2.0.14** sull'ultima serie 2.0 e **2.1.1** sulla
      nuova serie 2.1. Proposta:
      1. backend `GET /api/libtorrent/version` (installata + libreria linkata)
         e `POST /api/libtorrent/check-update` che interroga l'API GitHub
         releases di `arvidn/libtorrent` (e, se disponibile, `apt-cache policy`)
         confrontando le versioni;
      2. banner in Configurazione → Libtorrent: "libtorrent 2.0.11 — nuova
         versione 2.1.1 disponibile" con pulsante **Verifica aggiornamenti** e
         istruzioni di upgrade;
      3. upgrade assistito: da pacchetto (backports) o build da sorgente +
         `cargo build --release`; richiede root, quindi si aggancia all'helper
         privilegiato del primo punto. Il controllo deve degradare con
         grazia se manca rete/GitHub (nessun errore in UI).
      *(Implementati endpoint `POST /api/libtorrent/check-update` (GitHub
      releases + candidato `apt-cache policy`, confronto versioni robusto e
      degradazione con 502) e pulsante "Verifica aggiornamenti" in
      Configurazione → Libtorrent. L'upgrade automatico resta da fare.)*

- [x] **Build di sviluppo più rapida**. Il profilo `release`
      (`codegen-units=1` + LTO, su 4 CPU) impiegava 2–3 minuti per modifica;
      con un host a 4 core e senza `mold`/`lld` il collo di bottiglia è tutto
      in compilazione/link. Aggiunto `[profile.fast]` (`lto=false`,
      `codegen-units=16`, `opt-level=2`, incremental) e `scripts/build-fast.sh`
      (rileva `mold`/`lld` e altrimenti suggerisce l'installazione). Misurato:
      build a freddo 3m49s, **rebuild incrementale di `web.rs` ~3s** contro
      ~2–3 min di `release`. Il deploy resta su `release`. `mold` installato e
      attivato globalmente per il target host via `.cargo/config.toml`
      (`-C link-arg=-fuse-ld=mold`, con scope che lascia intatte le build wasm).
      `scripts/clean.sh` per lo spazio.

- [x] **Dashboard da rifare (testata e blocco rete)**. Problemi segnalati:
      - il blocco in alto (`.topbar` / `.top-actions`) va su **tre righe** e
        occupa molto spazio: "Prossimo ciclo", selettore **lingua** e i
        **pulsanti** (font A−/Testo/A+, tema, Esegui ciclo, Aggiorna, Backup,
        badge modalità, pill stato) devono stare **tutti su una sola riga**,
        compatti;
      - il riquadro **"Rete e download attivi"** duplica i valori **Download /
        Upload** che sono già mostrati nel **grafico** (sparkline): togliere le
        due stat tile e lasciare il grafico con la legenda ↓/↑;
      - valutare l'accorpamento delle azioni ciclo ridondanti (la testata ha
        "Esegui ciclo" e la dashboard ha di nuovo la barra "Avvia ciclo" con
        Tutto/Serie/Film/Fumetti/Backup).
      *(Riferimenti: `ui/app/src/lib.rs` — `topbar` ~748-793, toolbar ciclo
      ~1091-1098, pannello rete ~1171-1199.)*
      *(Fatto: `.topbar`/`.top-actions`/`.mode-strip` con `flex-wrap: nowrap` e
      scroll orizzontale; rimossi "Esegui ciclo" e "Backup ora" dalla testata
      (duplicati in dashboard/Manutenzione); rimosse le tile Download/Upload dal
      pannello "Rete e download attivi" lasciando il grafico con legenda ↓/↑.)*

- [x] **Seed infinito / ratio infinito per torrent (come legacy)**. Il backend lo
      supporta già: per torrent `seed_ratio`/`seed_days` = **-1 → politica
      globale**, **0 → infinito**, **>0 → limite esplicito**
      (`models.rs` doc, `web.rs` `enforce_seed_policy`/`remove_completed`).
      Manca però l'esperienza utente:
      - nel dettaglio torrent → tab **Limiti** i campi "Ratio seed" e "Giorni
        seed" hanno solo il placeholder "-1 = globale": aggiungere un controllo
        esplicito **"Seed infinito"** (checkbox/pulsante che imposta 0) e
        chiarire "-1 = globale · 0 = infinito";
      - **resa grafica** come legacy nella schermata **Scarico**: badge/etichetta
        "seed ∞" sulla riga del torrent (e nel dettaglio), così l'infinito è
        visibile senza aprire i limiti.
      *(Riferimenti: `ui/app/src/lib.rs` tab Limiti ~2404-2410, colonna/badge in
      lista torrent; `src/web.rs` `set_torrent_limits` ~5082, `TorrentView.seed_ratio`.)*
      *(Fatto: la tab Limiti carica i valori correnti all'apertura, ha il
      checkbox **"Seed infinito"** (imposta ratio/giorni a 0) e placeholder
      "-1 = globale · 0 = infinito"; la lista torrent mostra il badge
      **"seed ∞"** e il dettaglio mostra "Limite seed: ∞ (infinito)".)*
      **Bug corretto**: salvando i limiti con i campi Download/Upload a `-1`
      la UI inviava `-1 * 1024 = -1024` e il backend rispondeva *"limits must be
      between -1 and 2147483647 bytes/s"* (compariva anche impostando "Giorni
      seed" a 0). Ora i valori negativi restano `-1`.
      **Visibilità ∞ e stato**: il badge "seed ∞" era nella cella del nome
      (classe `truncate`) e veniva tagliato per i titoli lunghi; ora è mostrato
      nella **colonna Stato al posto di "In pausa"** per i torrent con seed
      infinito. Inoltre un torrent completato con seed infinito che era rimasto
      in pausa viene **ripreso automaticamente** (`torrent resumed for infinite
      seeding`), perché marcare "in pausa" un seed infinito non ha senso.

- [x] **Dashboard — rete su una sola riga**: **Load average**, **CPU sistema**,
      **RAM processo** e il **grafico** ora stanno su una riga
      (`.net-panel { grid-template-columns: repeat(3,1fr) minmax(0,1.6fr) }`),
      con fallback a 3 colonne + grafico a piena larghezza sotto i 1200px e a
      colonna singola sotto i 900px.
