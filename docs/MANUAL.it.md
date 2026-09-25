# Rextto — Manuale utente

Questo manuale descrive l'uso quotidiano di Rextto dalla sua interfaccia web.
La UI è disponibile in **italiano** e **inglese** (selettore lingua in alto); qui
sono usate le etichette italiane, la versione inglese è in
[`MANUAL.en.md`](MANUAL.en.md).

- [1. Primo avvio](#1-primo-avvio)
- [2. Dashboard](#2-dashboard)
- [3. Scarico](#3-scarico)
- [4. Serie TV](#4-serie-tv)
- [5. Film](#5-film)
- [6. Esplora, Archivio, Fumetti](#6-esplora-archivio-fumetti)
- [7. Configurazione](#7-configurazione)
- [8. Integrazioni](#8-integrazioni)
- [9. Manutenzione](#9-manutenzione)
- [10. Salute, Log, Grafici](#10-salute-log-grafici)
- [11. Notifiche](#11-notifiche)
- [12. Risoluzione problemi](#12-risoluzione-problemi)

---

## 1. Primo avvio

Rextto gira come un unico servizio. Apri la UI all'indirizzo `http://<host>:5000`.

- Se non esiste ancora una data directory, completa la procedura di setup iniziale.
- **Attivo vs dry-run**: in dry-run non partono download reali; abilita la
  *modalità attiva* in *Configurazione → Daemon* solo quando sei pronto.
- Aggiungi serie/film da **Esplora** (TMDB) oppure da **Serie TV / Film → Aggiungi**.

## 2. Dashboard

- **Ricerca manuale globale** — cerca in archivio + indexer + motori web.
- **Pulsanti ciclo** — avvia un ciclo completo o di un solo dominio (Serie, Film,
  Fumetti) o un backup immediato.
- **Card statistiche** — serie/film configurati, file scaricati, spazio libero,
  magnet in archivio, torrent in sessione, gruppi visti dai feed.
- **Rete e download attivi** — CPU/RAM e sparkline di rete in tempo reale.
- **Consumo e dischi**, **prossime uscite**, **ultimi download**,
  **attività recente** e **ultimi trovati nelle sorgenti**.

## 3. Scarico

- **Aggiungi** un magnet/URL `.torrent` oppure carica un file `.torrent`;
  opzionalmente percorso di salvataggio, “Scarica subito” e “Non rinominare”.
- **Card sessione**: velocità download/upload, numero torrent e peer.
- **Filtro per tag** e **limiti di velocità temporanei** (DL/UL per N minuti).
- **Colonne tabella**: Nome, Stato, Progresso, ↓, ↑, ETA, Peers, Ratio — clicca
  l'intestazione per ordinare. **Azioni bulk**: pausa, riprendi, recheck, rimuovi.
- **Azioni riga**: pausa/riprendi, recheck, dettagli, rimuovi.
- **Dettagli** (tab Generale, Tracker, Contenuto, Peers, Limiti, Storage):
  copia magnet, limiti per torrent/giorni di seed, reannounce, pin, riavvia,
  segna come fallito, sposta storage. In **Generale** trovi anche **Esporta
  .torrent**, **Super seeding** e l'aggiunta/rimozione di **web seed**; in
  **Tracker** puoi modificare l'intera lista (`tier|url` per riga); in
  **Contenuto** imposti la **priorità per file** (Salta/Normale/Alta/Massima).
- **Storico download**: elenca i download conclusi (nativi e migrati). Colonne:
  nome (con badge **NAS** quando il file è archiviato), tipo/stagione/episodio,
  **tag NAS** (regola di cartella), score, stato (*Completato*), **percorso
  libreria/NAS** e data di conclusione.

### Torrent stalled

Quando un torrent non aumenta i byte completati per il tempo configurato, passa
allo stato **stalled**: Rextto lo mette in pausa anche in libtorrent, quindi non
occupa più gli slot attivi. Il torrent resta nella sessione; al tentativo
successivo viene ripreso e riannunciato. I valori sono in *Configurazione →
libtorrent*:

- **Considera stalled dopo** — default 60 minuti;
- **Retry stalled** — default 60 minuti;
- **Rimozione stalled** — default 20160 minuti (14 giorni), `0` = mai.

La presenza di peer senza aumento dei byte non resetta il timer. Nel log cerca
`DOWNLOAD STALLED`, `stalled torrent resumed and reannounced` e, solo dopo il
limite finale, `DOWNLOAD FAILED — stalled`.

## 4. Serie TV

Aggiungi una serie via ricerca TMDB o manualmente (titolo, qualità, lingue,
stagioni, alias, esclusioni, percorso NAS, sottotitoli, timeframe).

- La lista mostra Nome, Stagioni, Qualità, Lingua, Episodi (scaricati/totali),
  Completezza, Ultimo download; filtrala con la casella di ricerca.
- Azioni bulk: imposta lingua, elimina selezionate.
- **Dettaglio serie**: locandina/trama/cast, badge (anno, rete, stato,
  completezza) e, se configurate, un badge **“stagioni disattivate”**.
- Azioni: cerca mancanti, scansiona archivio, aggiorna da TMDB, anteprima/esegui
  rinomina, modifica.
- **Episodi**: accordion per stagione; per ogni episodio puoi cercare, copiare il
  magnet, ignorare/riattivare, forzare, riscaricare o eliminare; la ricerca
  manuale segnala i risultati già presenti nel feed.

## 5. Film

- Tab **Monitorati / Scaricati**; colonne ordinabili (nome, anno, qualità, lingua).
- L'editor gestisce qualità, lingua base, sottotitoli, esclusioni e fino a **tre
  lingue richieste** con flag “obbligatoria” per riga.
- **Dettaglio film**: locandina, trama, cast, modifica, riscarica, **Cerca subito**
  e tabella dei “migliori trovati” dalle sorgenti.

## 6. Esplora, Archivio, Fumetti

- **Esplora** — TMDB: tendenze oggi/settimana, popolari, più votati, in
  programmazione, prossime uscite; ricerca TMDB; ricerca release generica;
  aggiunta alla libreria.
- **Archivio** — ricerca full-text delle release passate con paginazione, accoda
  in blocco, copia magnet, elimina; tab **Serie/Film dal feed**.
- **Visti dai feed** — ogni release vista nelle sorgenti, raggruppata per titolo
  (film/serie) con numero, risoluzione e score migliori; espandi un gruppo per
  accodare una singola release. Popolata ad ogni ciclo, anche per titoli non
  monitorati.
- **Fumetti** — Esplora GetComics + quick add, lista monitorati, estrazione link
  da un post, impostazioni weekly pack e storico con reinvia/elimina/forza.

## 7. Configurazione

Tab: **Daemon, Sorgenti, libtorrent, Punteggi, Rinomina, Avanzate, Acquisizione,
Notifiche, Percorsi, Traduzioni**. Le modifiche non salvate sono evidenziate con
la barra “Salva tutte”.

- **Sorgenti** — lista feed RSS, indexer (Jackett/Prowlarr) con pulsante
  *Verifica*, URL FlareSolverr + test, motori web, filtri contenuto, blacklist.
- **libtorrent** — connessioni/prestazioni, protocolli/tracker, sicurezza/proxy,
  RAM disk e porte, limiti di velocità e scheduler; applica/ottimizza/verifica
  aggiornamenti. In *Sicurezza, proxy e rete* l'**interfaccia VPN (killswitch)**
  vincola ascolto e traffico in uscita a una scheda scelta (es. `tun0`, `wg0`);
  l'elenco è letto dal server e la modifica si applica dopo il riavvio.
  Nella sezione RAM disk Rextto mostra i `tmpfs`/`ramfs` disponibili e consente
  di sceglierne uno. Se non c'è un percorso configurato, il pulsante dedicato
  crea `/dev/shm/rextto` e lo configura automaticamente. Il contenuto di
  `/dev/shm` è temporaneo e viene perso al riavvio della macchina. La scelta di
  un percorso calcola automaticamente dimensione massima per torrent, margine
  libero e spazio minimo; i valori restano modificabili. Il pulsante **Testa
  porte** verifica inoltre il bind locale delle porte indicate: non sostituisce
  il controllo del port-forwarding sul router/firewall.
- **Punteggi** — pesi per categoria, gruppi custom e simulatore live. Il punteggio
  effettivo è unico per acquisizione, ricerche, upgrade, post-processing,
  archivio e rescore; comprende anche bonus dimensione e, per i film, sottotitoli
  preferiti. Usa **Manutenzione → Ricalcola scoring** dopo aver cambiato i pesi.
- **Rinomina** — abilita rinomina, editor del template con token e anteprima,
  chiavi TMDB/TVDB, lingua, soglie di upgrade, token API. La verifica recupera il
  token sorgente dal titolo originale della release nel database ed elimina i
  blocchi placeholder vuoti (`[]`), così un `[WEB-DL]` perso viene ripristinato
  invece di restare `unknown`.
- **Avanzate** — spazio libero minimo, retention cestino, retention archivio,
  pagine feed, intervallo verifica rinomina, sposta episodi, flag di debug.
- **Acquisizione** — delay prima del download (serie/film, con bypass per
  punteggio alto), intervallo di housekeeping, **Cartelle osservate**: Rextto
  controlla le cartelle indicate e aggiunge i file `.torrent`/`.magnet` copiati,
  aspettando due rilevazioni stabili prima di leggerli e ritentando gli errori con
  backoff fino alla riuscita; li rimuove (o li rinomina `.imported`) dopo
  l'aggiunta; **Sorgenti in
  backoff**, con livello, scadenza, ultimo errore e reset per singola sorgente;
  e il **backfill MediaInfo** automatico (file per volta e intervallo
  configurabili), oltre al pulsante in Manutenzione per una scansione immediata.
  L'**housekeeping non è la pulizia dell'Archivio**: mantiene ordinato il
  database, ma non elimina i file della libreria, i download completati o le
  release elencate nell'Archivio. Il campo *riepiloghi cicli acquisizione
  conservati* riguarda solo le statistiche dei cicli: con valore `200`, il
  201° ciclo elimina il riepilogo più vecchio, non la release scaricata.
  *Visti nel feed* elimina solo le righe storiche dei feed oltre il numero di
  giorni indicato (`0` = nessuna pulizia); *storico download* elimina solo le
  righe dei torrent già rimossi (`0` = conserva). A ogni esecuzione vengono
  inoltre eliminati automaticamente torrent in errore oltre 7 giorni, log dei
  gap oltre 30 giorni, backup di upgrade oltre 30 giorni e backoff delle
  sorgenti scaduti. Alla fine i database vengono compattati con `VACUUM`.
  La retention dell'Archivio è separata e si trova in **Configurazione →
  Avanzate**.
- **Percorsi** — root libreria, cestino, cartelle download/temp/RAM disk, regole
  per tag. Il percorso RAM disk selezionato resta configurato, ma la directory
  creata sotto `/dev/shm` va ricreata dopo un riavvio.

I controlli di sanità delle release sono **automatici** e non configurabili:
sottotitoli hardcoded (`HC`) e dimensioni assurde (una soglia per risoluzione
derivata da un archivio reale) vengono rifiutati, con una riga a `INFO` nel log
che riporta titolo, risoluzione, dimensione e motivo solo per titoli monitorati;
le release non pertinenti vengono scartate senza log. Anche la protezione dagli
episodi vecchi è fissa: Rextto non riscarica un episodio precedente fuori dai
buchi riconosciuti quando possiede già episodi successivi (gap-fill e azioni
manuali restano sempre permessi). Per congelare un titolo usa **Consenti
aggiornamenti** nella scheda di serie/film.

I dati reali dei file (`ffprobe`: HDR, codec, audio, lingue) sono salvati per
episodio/film e **usati nei confronti di upgrade**: il file archiviato viene
letto com'è davvero, non solo dal nome. Sono additivi (non declassano mai un
file). I file nuovi vengono analizzati al completamento; gli altri vengono
coperti dal **backfill MediaInfo** incrementale schedulato (o dal pulsante in
Manutenzione per una scansione immediata). Se `ffprobe` non è installato il
backfill si mette in pausa da solo.

## 8. Integrazioni

- **Trakt / Simkl** — credenziali, flussi PIN/OAuth, import watchlist, calendario,
  scrobble/segna come visto.
- **Jellyfin / Plex** — URL + token del server e pulsante di aggiornamento
  libreria.
- **Hook eventi** — esegue un programma esterno su eventi Rextto
  (`download_started`, `torrent_completed`, `season_pack_completed`,
  `torrent_error`, …). I campi accettano segnaposto come `{title}`, `{hash}`,
  `{path}`, `{series}`, `{episode}`; gli stessi valori sono esposti come variabili
  d'ambiente `REXTTO_*`. I programmi sono eseguiti senza shell e con timeout di
  default pari a 60 secondi (massimo 24 ore; anche `0` usa il default).

## 9. Manutenzione

- Backup immediato, pulisci cestino, ricalcola scoring, scansiona archivi,
  **Aggiorna MediaInfo** (analizza con `ffprobe` i file archiviati senza dati e
  li salva) e riavvia il servizio.
- **Pulizia cestino** dalla toolbar **Manutenzione** — è forzata e rimuove subito
  il contenuto selezionato. La pulizia avviata dal pannello **Trash** rispetta
  invece il periodo di retention configurato.
- **Duplicati video** — *Anteprima duplicati* e *Pulisci duplicati* trovano i
  file video chiaramente inferiori (risoluzione strettamente più bassa) rimasti
  accanto alla versione migliore nella stessa cartella — es. il vecchio 480p
  accanto al nuovo 1080p — e li spostano nel cestino. Il controllo usa il nome
  del file e i dati tecnici dichiarati nel nome: non confronta il contenuto e
  non calcola hash. A **pari risoluzione**
  viene tenuta la versione nella lingua preferita (*Configurazione → Rinomina →
  lingua predefinita*) e il duplicato che dichiara esplicitamente un'altra lingua
  va nel cestino; i file senza tag lingua restano intatti. I file dei torrent
  ancora in sessione sono protetti e le sottocartelle svuotate vengono rimosse.
  La verifica di rinomina esegue la stessa pulizia automaticamente e la pulizia
  degli upgrade ora rispetta anche i motivi "forti" (risoluzione/sorgente/HDR/
  repack).
- **Ripristina sorgente** — *Ripristina sorgente nei nomi* rimette il token
  `[Source]` (WEB-DL, HDTV, BluRay…) nei nomi archiviati che l'hanno perso,
  recuperandolo dal titolo originale della release nel database. Non inventa la
  sorgente: se è sconosciuta il file resta invariato. Prima l'anteprima, poi
  l'esecuzione; nessun riscaricamento.
 - **Database**: prune per cicli/età errori, **retention dei "visti dai feed"**
   (giorni; 0 conserva tutto), prune per keyword con elenco delle righe
   corrispondenti e **VACUUM / ANALYZE** su tutti i database.
   La pulizia dei "visti" rimuove solo righe storiche dei feed, non file o
   download; applica anche la pulizia standard degli ultimi 50 cicli e degli
   errori torrent più vecchi di 7 giorni.
- **Backup**: retention, schedulazione (manuale, ogni N ore o a un orario fisso
  giornaliero HH:MM), FTP (host/utente/percorso) con **Test FTP** (verifica
  connessione, percorso e upload di prova), copia su cartella cloud/sync, invio
  Telegram, elenco backup. Lo snapshot contiene database e configurazione, non
  i media né lo stato della sessione torrent.

## 10. Salute, Log, Grafici

- **Salute** — stato processo/sistema, runtime (3 colonne), stato del servizio
  Rextto, **raggiungibilità degli indexer**, permessi percorsi, stato sorgenti,
  ultimi errori, dischi.
- **Log** — stream SSE live con filtro testuale, numero righe e segui/pausa.
  Le righe sono in inglese e in formato esplicito: `data ora  LIVELLO [componente]
  messaggio · campo: valore`, con parole chiave evidenziate (NAS, download,
  sorgenti, filtri, errori). Ogni ciclo stampa un **SOURCE REPORT** con l'esito
  di ogni sorgente, le decisioni dei filtri, e gli eventi di download (avvio,
  metadati, spostamento su NAS, completamento). I messaggi torrent includono
  sempre nome o titolo leggibile; l'hash è solo un campo tecnico per correlare
  gli errori.
- **Grafici** — sparkline CPU/RAM/download/upload/disco/RAM disk e consumo
  giornaliero.
- **Attività** — eventi torrent recenti e download.

## 11. Notifiche

Configura Telegram, e-mail (SMTP) o un webhook (con segreto HMAC) e invia un
test. Le notifiche di completamento includono dimensione, tempo di download e
velocità media.

## 12. Risoluzione problemi

- **Una sorgente non risponde** — controlla *Configurazione → Sorgenti → Verifica*
  e il pannello stato sorgenti; i siti protetti da Cloudflare richiedono un
  FlareSolverr funzionante.
- **Non scarica nulla** — verifica la *modalità attiva*, che serie/film siano
  abilitati, e controlla filtri qualità/lingua e il limite di spazio libero.
- **Un torrent è stalled** — controlla i tre valori in *Configurazione →
  libtorrent*. Il torrent è intenzionalmente pausato e fuori dalla coda attiva;
  attendi il retry oppure usa **Riprendi/Riavvia** manualmente.
- **Una cartella osservata non importa il file** — lascia il file con estensione
  `.torrent` o `.magnet`; Rextto aspetta che dimensione e timestamp restino
  stabili, poi ritenta automaticamente gli errori. Controlla il log del watcher.
- **Un file non viene rinominato** — conviene avere `mediainfo` installato (tag
  tecnici); controlla le impostazioni *Rinomina* e la chiave TMDB.
- **Il backup FTP fallisce** — usa *Test FTP*: indica il passo che fallisce
  (connessione, login, percorso remoto, upload, rimozione) e lo registra nel log.
- **Log** — vedi `data/rextto.log` (rotazione a 5 MB) o il viewer nella UI.
