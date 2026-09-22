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
  segna come fallito, sposta storage.
- **Storico download**: elenca i download conclusi (nativi e migrati). Colonne:
  nome (con badge **NAS** quando il file è archiviato), tipo/stagione/episodio,
  **tag NAS** (regola di cartella), score, stato (*Completato*), **percorso
  libreria/NAS** e data di conclusione.

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

Tab: **Daemon, Sorgenti, libtorrent, Punteggi, Rinomina, Avanzate, Notifiche,
Percorsi, Traduzioni**. Le modifiche non salvate sono evidenziate con la barra
“Salva tutte”.

- **Sorgenti** — lista feed RSS, indexer (Jackett/Prowlarr) con pulsante
  *Verifica*, URL FlareSolverr + test, motori web, filtri contenuto, blacklist.
- **libtorrent** — connessioni/prestazioni, protocolli/tracker, sicurezza/proxy,
  RAM disk e porte, limiti di velocità e scheduler; applica/ottimizza/verifica
  aggiornamenti. In *Sicurezza, proxy e rete* l'**interfaccia VPN (killswitch)**
  vincola ascolto e traffico in uscita a una scheda scelta (es. `tun0`, `wg0`);
  l'elenco è letto dal server e la modifica si applica dopo il riavvio.
- **Punteggi** — pesi per categoria, gruppi custom e simulatore live.
- **Rinomina** — abilita rinomina, editor del template con token e anteprima,
  chiavi TMDB/TVDB, lingua, soglie di upgrade, token API. La verifica recupera il
  token sorgente dal titolo originale della release nel database ed elimina i
  blocchi placeholder vuoti (`[]`), così un `[WEB-DL]` perso viene ripristinato
  invece di restare `unknown`.
- **Avanzate** — spazio libero minimo, retention cestino, retention archivio,
  pagine feed, intervallo verifica rinomina, sposta episodi, flag di debug.
- **Percorsi** — root libreria, cestino, cartelle download/temp/RAM disk, regole
  per tag.

## 8. Integrazioni

- **Trakt / Simkl** — credenziali, flussi PIN/OAuth, import watchlist, calendario,
  scrobble/segna come visto.
- **Jellyfin / Plex** — URL + token del server e pulsante di aggiornamento
  libreria.

## 9. Manutenzione

- Backup immediato, pulisci cestino, ricalcola scoring, scansiona archivi,
  importa dati legacy, riavvia il servizio.
- **Duplicati video** — *Anteprima duplicati* e *Pulisci duplicati* trovano i
  file video chiaramente inferiori (risoluzione strettamente più bassa) rimasti
  accanto alla versione migliore nella stessa cartella — es. il vecchio 480p
  accanto al nuovo 1080p — e li spostano nel cestino. A **pari risoluzione**
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
- **Backup**: retention, schedulazione (manuale, ogni N ore o a un orario fisso
  giornaliero HH:MM), FTP (host/utente/percorso) con **Test FTP** (verifica
  connessione, percorso e upload di prova), copia su cartella cloud/sync, invio
  Telegram, elenco backup.

## 10. Salute, Log, Grafici

- **Salute** — stato processo/sistema, runtime (3 colonne), stato del servizio
  Rextto, **raggiungibilità degli indexer**, permessi percorsi, stato sorgenti,
  ultimi errori, dischi.
- **Log** — stream SSE live con filtro testuale, numero righe e segui/pausa.
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
- **Un file non viene rinominato** — conviene avere `mediainfo` installato (tag
  tecnici); controlla le impostazioni *Rinomina* e la chiave TMDB.
- **Il backup FTP fallisce** — usa *Test FTP*: indica il passo che fallisce
  (connessione, login, percorso remoto, upload, rimozione) e lo registra nel log.
- **Log** — vedi `data/rextto.log` (rotazione a 5 MB) o il viewer nella UI.
