# Audit parità UI Rextto ↔ legacy

Data: 2026-09-20
Metodo: inventario completo di legacy (`templates/index.html`, `static/js/app.js`,
`extto_web.py`) e di Rextto (`ui/app/src/lib.rs`, `src/web.rs`), con verifica dei
gap direttamente sul codice Rextto (grep mirati). Non solo checklist.

> Nota: questo è l’inventario iniziale dei gap UI. Per lo stato verificato
> corrente (build, API, UI isolata e runtime) consultare
> [`FUNCTIONAL_AUDIT.md`](FUNCTIONAL_AUDIT.md); la checklist operativa resta
> [`WORKPLAN.md`](WORKPLAN.md).

Legenda: ❌ assente in Rextto · ⚠️ parziale · 🟢 solo UI da aggiungere (API già pronta)

legacy ha 18 view; Rextto ha 19 pagine ma con perimetri diversi.

---

## A. Intere sezioni/funzionalità assenti

- ❌ **aMule / ed2k** — in legacy è una sezione completa con 8 tab (Download,
  Ricerca, Server, Condivisi, Upload, Impostazioni, Statistiche, Log), barra rete
  ed2k/Kad/server, "Recupera .part", import `server.met`, cartelle condivise
  ricorsive, gap-filling eD2k. In Rextto non esiste (nessuna traccia nel codice).
- ❌ **Dettaglio film stile Radarr** (view dedicata): locandina, trama, "Cerca
  subito", badge tecnici, sezione "Migliori Trovati" dal feed.
- ❌ **Dettaglio serie stile legacy**: header con locandina/rete/trama e sezione
  "Ultimi trovati nel feed" (in Rextto esiste il `SeriesPanel`, ma senza queste parti).
- ❌ **Manuale completo**: legacy ha ~50 sezioni; Rextto ha una schermata statica
  "Manuale rapido".
- ❌ **Configurazione client esterni** (qBittorrent, Transmission, aria2, rqbit).
  Nota: Rextto per progetto usa solo libtorrent (vedi ACCEPTANCE).

## B. Dashboard

- ❌ Grafico live rete (CPU/RAM/↓/↑) + sparkline dei download (Rextto mostra solo numeri).
- ❌ Countdown "prossimo ciclo" (next-run timer) nell'header.
- ❌ Lista dischi con spazio (Rextto: solo "spazio libero" + trash).
- ⚠️ Stat card con sotto-contatori e delta (Rextto: forma ridotta).

## C. Scarico / Torrent

- ❌ Barra statistiche sessione (↓/↑ totali, n. torrent, n. connessioni).
- ❌ Filtro per tag.
- ❌ Selezione multipla + azioni bulk (Seleziona tutti, Pausa/Riprendi/Recheck/
  Rimuovi selezionati).
- ❌ Colonne tabella mancanti: **ETA, Peers, Ratio** (Rextto: Nome/Stato/Progresso/↓/↑/Azioni).
- ❌ Ordinamento colonne cliccando l'intestazione.
- 🟢 **"Segna come fallito"** (`markTorrentFailed`): API `POST /api/torrents/{hash}/mark_failed`
  già presente, manca il pulsante.
- ❌ **"Riavvia torrent"** (restart): assente UI (e probabilmente API).
- ❌ Modale "Aggiungi magnet" con **percorso di salvataggio**, **"scarica subito"**,
  **"non rinominare"** (Rextto: solo magnet/URL + file .torrent).
- ❌ Conferma "no-rename".
- ⚠️ Dettaglio torrent: Rextto ha 6 tab (anche Limiti/Storage, meglio di legacy) ma
  mancano **grafico velocità** e **"Copia magnet"**.

## D. Serie TV

- ❌ Filtro sull'elenco già monitorato (l'input presente in Rextto serve per cercare
  su TMDB, non per filtrare la tabella).
- ❌ Colonne mancanti: **Ep.** (n. episodi), **Ultimo** (ultimo download),
  **completezza %** (legacy usa `/api/series/completeness`).
- ❌ Azioni bulk (imposta lingua a più serie, elimina multiple).
- ❌ Opzione **sottocartelle per stagione** nell'editor (assente anche nel backend).
- ❌ Per-episodio: **cerca manuale**, **copia magnet**, **feed matches** ("Ultimi
  trovati"), colonna "Aggiunto al client".
- ⚠️ Editor: lingua/sottotitoli con preset+custom (legacy) vs semplici select (Rextto).

## E. Film

- ❌ Filtro sull'elenco già monitorato.
- ❌ Colonne **Qualità** e **Lingua** nella lista "Monitorati" (Rextto: Nome/Anno/Stato).
- ❌ **Lingue multiple (fino a 3) + flag "obbligatoria"**: il backend ha
  `language_requirements`, la UI espone una sola "Lingua".
- ❌ Ordinamento colonne.
- ❌ Dettaglio: locandina/trama/pulsante TMDB e sezione "Migliori Trovati"/"Cerca subito".

## F. Esplora

- ❌ Categorie TMDB: **top_rated, now_playing, upcoming** (Rextto ha solo tendenze e
  popolari; l'API `tmdb.rs` implementa solo `trending` e `popular`).
- ✅ Il resto (griglia, "Già in lista", popup qualità/lingua/stagioni) c'è.

## G. Archivio

- ❌ Tab **"Film dal Feed"** e **"Serie TV dal Feed"** (assenti anche nel backend).
- ❌ Checkbox **"Seleziona tutti"**.
- ❌ Azioni bulk **"copia"** e **"elimina selezionati"** (Rextto ha solo "Accoda selezionate").
- ❌ Azione **"cerca su TMDB"** da una voce d'archivio.
- ❌ Pulsante **"Aggiungi magnet"** in archivio (presente solo in Scarico).

## H. Fumetti

- ❌ Tab **"Esplora"** con ricerca su GetComics + griglia + quick-add (Rextto permette
  solo l'aggiunta manuale per titolo/tag URL; manca anche l'API `comics/search`).
- ❌ Storico: **"Reinvia"** e **"Elimina"**.
- ❌ Weekly pack: lista dei pack con stato + **"Forza"** (Rextto: solo impostazioni + ricerca).
- ❌ Modale di modifica monitorato (data inizio, cartella destinazione).

## I. Configurazione

- ❌ Manca la tab **"Avanzate"** con molte opzioni non esposte:
  `move_episodes`, `rename_verify_interval`, `cleanup_min_score_diff`,
  `upgrade_min_score_diff`, `trash_retention_days`, `default_language`,
  `min_free_space_gb`, `stop_on_old_page_threshold`, flag `debug_*`,
  `archive_cleanup_enabled`, `archive_max_age_days`, `archive_keep_min`,
  `tmdb_language`.
- ❌ **Punteggi**: mancano scoring per **gruppi custom** e **simulatore/calcolatore**
  del punteggio (legacy li ha).
- ❌ **Sorgenti**: mancano preset URL (extto/corsaro/knaben/rss/tgx), lista "wanted",
  lista custom score.
- ❌ **Libtorrent – parametri avanzati** non esposti: queue disk, send buffer,
  max peer list, slow dl/ul, preallocate, disable COW, extra trackers, proxy
  type/user/pass, stall timeout, dynamic queue min/max, ramdisk threshold/margin,
  port min/max.
- ❌ **RAM disk**: manca check/dimensioni filesystem.
- ❌ **Selettore lingua interfaccia** nell'header (esiste l'editor traduzioni, non lo
  switch di lingua attiva runtime — `🟢` API `/api/i18n/active` già presente).
- ❌ **Browser handler** (installazione handler magnet/.torrent).
- ⚠️ Salvataggio: Rextto salva per singolo campo; legacy ha un "Salva tutte le
  impostazioni" unico con banner modifiche non salvate.

## J. Manutenzione

- ❌ **VACUUM / ANALYZE** (ottimizzazione SQLite). Assenti anche le API.
- ❌ Pulizia profonda con **filtri script/porn** e selezione per ID.
- ❌ **Gestione servizi** systemd (riavvia servizio, stato/start/stop aria2/rqbit/amule).
- ⚠️ Backup: presente retention + FTP + Telegram + intervallo in ore; manca lo
  scheduling **giornaliero/settimanale** e il tipo cloud.
- ✅ Blocklist: in Rextto è una pagina dedicata (ok).

## K. Grafici

- ❌ Grafico live CPU/RAM/disco/ramdisk/↓/↑ (Rextto ha solo la tabella consumo
  giornaliero). Nessuna libreria di grafici integrata.

## L. Salute

- ❌ Mancano: lista dischi, lista indexer (ping Jackett/Prowlarr), permessi cartelle,
  lista servizi, ultimi errori log, dettaglio consumo 7/30/totale.
- Rextto ha: 4 metriche + runtime.

## M. Log

- ⚠️ Rextto usa polling ogni 3 s; legacy ha **stream SSE**. Manca l'input del numero
  di righe (Rextto fisso a 400) e lo streaming realtime.

## N. Integrazioni

- ❌ **Jellyfin / Plex**: nessuna UI (esistono solo le route di refresh).
- ⚠️ **Watchlist/Calendario Trakt e Simkl**: Rextto li scarica ma li mostra come
  **JSON grezzo** ("Output provider"); legacy ha tabelle con import selettivo.

## O. Trasversali / dettagli

- ❌ Notifiche toast (legacy `showToast`); Rextto usa notice inline.
- ❌ Ricerca manuale globale: manca la sezione **eD2k**.
- ❌ Countdown prossimo ciclo.
- (Irrilevante) easter egg arcade di legacy.

---

## Fuori scope dichiarato (da confermare)

- Client torrent esterni (qBittorrent, Transmission, aria2, rqbit) e aMule/ed2k:
  Rextto dichiara di usare **solo libtorrent**. Se restano fuori scope, ignorare
  A1/A5 e la parte "client" di I.

## Quick win (solo UI, API già disponibile)

1. "Segna come fallito" torrent (`mark_failed`).
2. Switch lingua interfaccia runtime (`/api/i18n/active`).
3. Jellyfin/Plex: pannello config + refresh (refresh già presente).
4. Feed status (`/api/feed/status`) — base per "Ultimi trovati".
5. Pulsante "Copia magnet" nel dettaglio torrent (il magnet è nei dati del torrent).
6. Colonne ETA/Peers/Ratio nella tabella torrent (i dati arrivano già da `/api/torrents`).
