# Rextto — Manuale utente

Questo manuale descrive l'uso quotidiano di Rextto dalla sua interfaccia web.
La UI è disponibile in **italiano** e **inglese** (selettore lingua in alto); qui
sono usate le etichette italiane, la versione inglese è in
[`MANUAL.en.md`](MANUAL.en.md).

## Come usare questa guida

Rextto è un demone unico: ricerca le sorgenti, valuta le release, gestisce
libtorrent, rinomina i file e li archivia. Non è necessario avviare componenti
separati per il funzionamento normale.

Il percorso consigliato per una nuova installazione è:

1. configurare percorsi e sorgenti;
2. lasciare il demone in **dry-run**;
3. aggiungere un solo titolo di prova;
4. eseguire una ricerca o un ciclo manuale;
5. controllare Salute e Log;
6. abilitare la modalità attiva solo dopo aver verificato i risultati.

Il manuale distingue sempre tra:

- **ricerca manuale**: serve a ispezionare risultati e accodare una scelta;
- **ciclo automatico**: cerca i titoli monitorati e decide cosa scaricare;
- **archivio**: i file già importati o archiviati nella libreria;
- **sessione torrent**: i download ancora gestiti da libtorrent.

### Checklist iniziale

Prima di abilitare i download reali verifica:

- `http://<host>:5000` è raggiungibile;
- *Salute* non segnala problemi di permessi o spazio;
- la cartella temporanea è scrivibile;
- la cartella libreria/NAS è montata e scrivibile dall'utente del servizio;
- almeno una sorgente risponde a *Verifica*;
- una ricerca manuale restituisce release coerenti;
- in dry-run Rextto non ha prodotto errori inattesi.

Se usi un NAS, prova prima a creare un file nella destinazione con lo stesso
utente che esegue `rextto.service`. Un percorso visibile dalla shell dell'utente
personale può non essere visibile al servizio systemd.

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

### Procedura consigliata per il primo ciclo

1. In *Configurazione → Percorsi* controlla cartella download, temporanea,
   libreria e cestino.
2. In *Configurazione → Sorgenti* aggiungi una sola sorgente funzionante e premi
   **Verifica**. Aggiungi le altre solo dopo aver validato la prima.
3. Lascia disattivati i download reali e aggiungi una serie con una sola stagione
   o un film di prova.
4. Dalla Dashboard avvia il ciclo del dominio interessato.
5. Apri *Salute* e *Log*: devi vedere il ciclo, le sorgenti interrogate e il
   motivo per cui una release è stata accettata o scartata.
6. Esegui una ricerca manuale e controlla un risultato con **Perché non questa?**.
7. Quando percorsi, filtri e risultati sono corretti, abilita *Modalità attiva*.

In caso di dubbi non modificare contemporaneamente qualità, percorsi e sorgenti:
una modifica alla volta rende il problema riproducibile.

### Percorsi e responsabilità

Rextto usa percorsi con ruoli diversi:

| Percorso | Uso | Può essere temporaneo? |
|---|---|---|
| Download | dati dei torrent in sessione | no, finché il torrent è attivo |
| Temporaneo/incomplete | metadati e dati incompleti | sì, ma deve essere scrivibile |
| Libreria/NAS | file finali archiviati | no |
| Cestino | file rimossi durante upgrade/pulizia | sì, secondo retention |
| Cartella osservata | `.torrent`/`.magnet` da importare | sì, ma non durante la copia |

Non usare la cartella temporanea come libreria finale e non cancellare a mano i
file di un torrent ancora attivo: usa le azioni della sessione torrent.

### Riga di comando e aggiornamenti

Il demone gira normalmente nel servizio systemd. Eseguito direttamente
comprende:

- `rexttod --version` — versione installata, build number, marker di release e
  libtorrent inclusa;
- `rexttod --help` — riepilogo d'uso;
- `rexttod --update` — scarica e installa l'ultimo payload;
- `rexttod --config <file>` e `rexttod --dry-run` — usati dal servizio e per le
  prove locali.

**Aggiornamento.** L'installer e `rexttod --update` installano lo stesso
payload. `--update` scarica `rextto-linux-<arch>.tar.gz`, verifica il `.sha256`
pubblicato quando presente, prepara i file e poi sostituisce eseguibile, UI web,
`lib/` inclusa e `run.sh` con rename atomici. Dati e configurazione in
`REXTTO_DATA_DIR` (default `/var/lib/rextto`) non vengono mai toccati: un
download, un checksum o un'estrazione falliti lasciano l'installazione in
esecuzione invariata, e uno swap fallito viene ripristinato. Il marker `VERSION`
accanto all'eseguibile viene aggiornato ed è mostrato da `--version`.

- `--channel stable` / `--release <tag>` — scelgono la release da installare;
- `--install-dir <dir>` — installa altrove (default: la directory del binario);
- `--archive <file>` — installa da un archivio locale (offline);
- `--force` — reinstalla anche se la versione è invariata;
- `--no-restart` — non riavviare `rextto.service`.

Eseguito come root riavvia `rextto.service` automaticamente; altrimenti stampa il
comando `systemctl` esatto. L'unità systemd non viene sovrascritta, così le
personalizzazioni locali (utente, porte, percorsi) restano. Lo stesso payload è
il pacchetto Linux autonomo descritto nel README (*Pacchetto Linux autonomo*).

## 2. Dashboard

- **Ricerca manuale globale** — cerca in archivio + indexer + motori web.
- **Pulsanti ciclo** — avvia un ciclo completo o di un solo dominio (Serie, Film,
  Fumetti) o un backup immediato.
- **Card statistiche** — serie/film configurati, file scaricati, spazio libero,
  magnet in archivio, torrent in sessione, gruppi visti dai feed.
- **Rete e download attivi** — CPU/RAM e sparkline di rete in tempo reale.
- **Consumo e dischi**, **prossime uscite**, **ultimi download**,
  **attività recente** e **ultimi trovati nelle sorgenti**.

### Ricerca manuale dalla Dashboard

La ricerca manuale è utile per capire cosa vede Rextto prima di modificare una
configurazione o avviare un download:

1. inserisci un titolo o una query tecnica;
2. attendi che le sorgenti terminino o che il timeout segnali quelle lente;
3. confronta titolo, sorgente, qualità, dimensione e seed/peer;
4. usa **Filtra i risultati già caricati** per restringere localmente la lista;
5. apri **Perché non questa?** sui risultati interessanti;
6. usa **Accoda** solo dopo aver controllato il motivo e il confronto archivio.

Il filtro locale lavora sui risultati già ricevuti, inclusi titolo, sorgente e
campi tecnici della qualità. Scrivere nel filtro non interroga nuovamente gli
indexer e non modifica la query originale.

La ricerca manuale può mostrare release non idonee per permettere l’ispezione.
Il fatto che una release sia visibile non significa che il ciclo automatico la
scaricherebbe.

### Come leggere un ciclo

Un ciclo normale attraversa, in ordine, ricerca, filtri, confronto con archivio,
selezione e accodamento. Il numero di release trovate non equivale al numero di
download: una release può essere esclusa perché non monitorata, troppo vecchia,
bloccata, già presente o inferiore al file archiviato.

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
- Quando usi **Sposta storage**, il log registra richiesta, destinazione e
  accettazione del comando; l'esito finale viene scritto quando libtorrent
  completa o rifiuta lo spostamento. Con **Check** il log distingue comando
  avviato e controllo terminato, includendo stato e byte verificati.
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

### Stati e azioni consigliate

| Stato | Significato | Azione consigliata |
|---|---|---|
| In coda | registrato ma non ancora avviato | attendere il ciclo/sessione |
| Download | trasferimento in corso | controllare velocità e peer |
| Stalled | nessun aumento reale dei byte | attendere il retry automatico |
| Seeding | download completato, seed ancora attivo | lasciare il torrent o rimuoverlo secondo policy |
| Errore | il torrent ha riportato un errore | leggere il motivo prima di rimuoverlo |
| Archiviato/NAS | il file finale è stato copiato nella libreria | verificare il percorso, non cancellare la sorgente mentre è in uso |

**Rimuovi** agisce sulla sessione torrent e può chiedere se eliminare i file.
**Pulisci completati** è più selettivo: rimuove i torrent che hanno raggiunto i
limiti di seed. Un torrent rimosso dalla sessione non equivale necessariamente a
un file rimosso dalla libreria.

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
  L'anteprima di rinomina elenca i nomi *Vecchio → Nuovo* e ha un pulsante
  **Forza rinomina**: rielabora anche i file che superano il controllo rapido,
  ricalcolando il nome esatto dal template e dai metadati TMDB/TVDB; è utile
  quando un file sembra corretto ma non rispetta il template configurato.
- **Episodi**: accordion per stagione; per ogni episodio puoi cercare, copiare il
  magnet, ignorare/riattivare, forzare, riscaricare o eliminare; la ricerca
  manuale segnala i risultati già presenti nel feed.

### Aggiungere e configurare una serie

Per una serie nuova il metodo più sicuro è la ricerca TMDB:

1. apri **Esplora**, cerca il titolo e seleziona il risultato corretto;
2. controlla titolo, anno, rete e poster;
3. scegli qualità, lingua, sottotitoli e stagioni da monitorare;
4. imposta il percorso archivio solo se non vuoi usare quello globale;
5. salva in dry-run e controlla la pagina di dettaglio.

I campi più importanti sono:

| Campo | Effetto |
|---|---|
| Qualità | restringe le release accettabili e partecipa allo score |
| Lingua | richiede la lingua configurata quando è riconoscibile nel titolo |
| Sottotitoli | gestisce la preferenza senza accettare sottotitoli hardcoded |
| Stagioni | decide quali stagioni sono monitorate; gli intervalli sono supportati |
| Alias | aiuta a collegare titoli diversi alla stessa serie |
| Esclusioni | parole nel titolo che devono bloccare la release |
| Consenti aggiornamenti | abilita o blocca la sostituzione di file già archiviati |

Lascia le stagioni non ancora disponibili abilitate se vuoi che il calendario e
la ricerca dei mancanti continuino a funzionare. Usa *Ignora* solo per episodi
che non vuoi più cercare: riattivandoli tornano candidati nei cicli successivi.

### Episodi mancanti e pack

Da **Cerca mancanti** o dalla scheda episodio:

1. controlla che la stagione sia monitorata;
2. avvia la ricerca del singolo episodio o del dominio serie;
3. confronta release singole e season pack;
4. se un episodio è già sul disco ma non nel database, usa la scansione archivio;
5. usa **Forza** solo per un’azione manuale consapevole.

Rextto evita normalmente di riscaricare episodi vecchi quando possiede episodi
successivi, a meno che si tratti di un buco riconosciuto o di un upgrade reale.
Un season pack può riempire più episodi, ma il confronto con l’archivio resta
episodio per episodio.

## 5. Film

- Tab **Monitorati / Scaricati**; colonne ordinabili (nome, anno, qualità, lingua).
- L'editor gestisce qualità, lingua base, sottotitoli, esclusioni e fino a **tre
  lingue richieste** con flag “obbligatoria” per riga.
- **Dettaglio film**: locandina, trama, cast, modifica, riscarica, **Cerca subito**
  e tabella dei “migliori trovati” dalle sorgenti.

### Aggiungere e scegliere un film

1. cerca il film in **Esplora** e controlla anno e titolo originale;
2. aggiungilo alla libreria;
3. imposta qualità, lingua, sottotitoli ed eventuali esclusioni;
4. usa **Cerca subito** per vedere i risultati senza aspettare il ciclo;
5. apri **Perché non questa?** prima di accodare una release borderline.

Il film viene identificato usando titolo e anno quando disponibili. Evita di
creare duplicati con lo stesso film scritto in modi diversi: correggi i metadati
del film monitorato invece di aggiungerlo nuovamente.

Le lingue richieste possono essere opzionali o obbligatorie. Una lingua
obbligatoria deve essere presente perché la release sia ammessa; una preferenza
opzionale influenza la scelta e lo score senza trasformarsi automaticamente in un
blocco.

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
- **Perché non questa?** — nei risultati di ricerca puoi aprire una spiegazione
  read-only della release. Mostra esito, score e componenti dello score, regole
  superate o bloccanti, filtro sorgente, qualità/lingua/sottotitoli, blocklist,
  download attivi e confronto con il file già presente nell'archivio. Non accoda
  la release, non crea placeholder e non modifica il database. L'esito riguarda
  i controlli del candidato: la scelta finale del ciclo può dipendere anche da
  gap filling, smart episode, delay, spazio libero e confronto con altri
  candidati.
- **Fumetti** — in **Aggiungi fumetto** scrivi il titolo e premi **Trova**; scegli
  il risultato esatto di GetComics e poi conferma. Rextto salva il post scelto,
  il tag, la copertina e i metadati, e usa quel post per il download senza
  sostituirlo con un albo omonimo. Sono inoltre disponibili lista monitorati,
  estrazione link da un post, impostazioni weekly pack e storico con
  reinvia/elimina/forza.

## 7. Configurazione

Tab: **Daemon, Sorgenti, libtorrent, Punteggi, Rinomina, Avanzate, Acquisizione,
Notifiche, Percorsi, Traduzioni**. Le modifiche non salvate sono evidenziate con
la barra “Salva tutte”.

- **Sorgenti** — lista feed RSS, indexer (Jackett/Prowlarr) con pulsante
  *Verifica*, URL FlareSolverr + test, motori web, filtri contenuto, blacklist.
  Per Jackett puoi indicare l'URL base, ad esempio `http://host:9117`, e la
  relativa API key: Rextto costruisce l'endpoint Torznab
  `/api/v2.0/indexers/all/results/torznab/api`. Il controllo di salute usa
  `t=caps` con la stessa API key, quindi verifica l'API e non soltanto la home di
  Jackett. Nei risultati la sorgente può apparire come `jackett:NomeTracker`.
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
   release elencate nell'Archivio. Il campo *statistiche cicli di ricerca
   conservate* riguarda solo i contatori di ogni ciclo — release analizzate,
   candidate, download avviati, gap riempiti ed errori: con valore `200`, il
   201° ciclo elimina i contatori del ciclo più vecchio, non la release
   scaricata.
  *Visti nel feed* elimina solo le righe storiche dei feed oltre il numero di
   giorni indicato (`0` = nessuna pulizia); lo *Storico download* della sezione
   **Scarico** elimina solo le righe dei torrent già rimossi (`0` = conserva),
   senza toccare l'Archivio. A ogni esecuzione vengono
  inoltre eliminati automaticamente torrent in errore oltre 7 giorni, log dei
  gap oltre 30 giorni, backup di upgrade oltre 30 giorni e backoff delle
  sorgenti scaduti. Alla fine i database vengono compattati con `VACUUM`.
  La retention dell'Archivio è separata e si trova in **Configurazione →
  Avanzate**.
- **Percorsi** — root libreria, cestino, cartelle download/temp/RAM disk, regole
   per tag. Il percorso RAM disk selezionato resta configurato, ma la directory
   creata sotto `/dev/shm` va ricreata dopo un riavvio.
 - **Traduzioni** — pannello avanzato per esportare in YAML le traduzioni
   salvate per italiano o inglese, modificarle e reimportarle. Il formato è una
   mappa `chiave: valore`, ad esempio `"Testa porte": "Test ports"`.
   L'importazione aggiorna o aggiunge le chiavi presenti e non cancella quelle
  assenti dal file. Non cambia la lingua attiva: quella si sceglie dal
  selettore in alto. La UI traduce le stringhe a runtime e, se manca una
  traduzione, mostra la sorgente italiana.

### Configurare un indexer Torznab

Per Jackett:

1. crea o verifica almeno un indexer dentro Jackett;
2. copia la API key dalla pagina Jackett;
3. in Rextto premi **+ Jackett**;
4. inserisci URL base, ad esempio `http://jackett:9117`, e API key;
5. salva e premi **Verifica**;
6. controlla *Salute → Stato API*.

Rextto usa automaticamente `/api/v2.0/indexers/all/results/torznab/api` per
Jackett. Non aggiungere quel percorso se stai usando l'URL base. Se usi un
percorso Torznab personalizzato già completo, lascialo configurato come endpoint
esplicito. Prowlarr usa invece il proprio endpoint di ricerca e normalmente la
porta `9696`.

Un indexer può essere raggiungibile ma non sano: per esempio Jackett può
rispondere HTTP 200 con un errore Torznab dovuto a API key errata o nessun
indexer abilitato. In questo caso *Salute* mostra **Errore API** e il dettaglio;
il log del ciclo mostrerà anche il backoff della sorgente quando applicabile.

### Punteggio, filtri e “Perché non questa?”

I controlli principali sono indipendenti:

1. titolo monitorato;
2. blocklist e duplicati;
3. filtri globali e filtro della sorgente;
4. sanità della release, inclusi sottotitoli hardcoded e dimensione minima;
5. qualità, lingua, sottotitoli ed esclusioni del titolo;
6. confronto con file già archiviati e download attivi.

Lo score aiuta a scegliere tra candidati ammessi; non rende valida una release
che fallisce un filtro bloccante. Nel pannello **Perché non questa?**:

- **Superata** significa che quel controllo è passato;
- **Bloccante** indica un motivo sufficiente a rifiutare il candidato;
- **Informativa** indica un controllo che dipende dal contesto dell'intero ciclo;
- il confronto archivio mostra se si tratta di primo download o di possibile
  upgrade.

La spiegazione è una diagnosi, non una prenotazione: aprirla non crea righe
torrent, non accoda magnet e non cambia la configurazione.

### Ritardi e backoff delle sorgenti

Il delay di acquisizione può trattenere una release prima dell'avvio per dare
tempo a un risultato migliore. Un punteggio alto può bypassare il delay secondo
la configurazione. Il backoff delle sorgenti è diverso: viene attivato da errori
ripetuti e impedisce temporaneamente nuove richieste alla sorgente problematica.

In *Configurazione → Acquisizione* puoi vedere livello, scadenza e ultimo errore.
Usa il reset per singola sorgente dopo aver corretto la causa; non usarlo per
mascherare un'API key sbagliata, altrimenti il backoff ricomincerà.

### Configurare il NAS senza sorprese

Per una libreria su NAS:

1. monta il filesystem prima dell'avvio di Rextto;
2. assegna permessi di lettura/scrittura all'utente del servizio;
3. imposta spazio minimo prudente;
4. esegui una scansione archivio dopo aver copiato file esistenti;
5. controlla il percorso effettivo nella cronologia dopo il primo completamento.

Se il NAS non è montato, non sostituire temporaneamente il percorso con la root
locale senza aver capito l'effetto: potresti archiviare file nel posto sbagliato.
Meglio correggere il mount e lasciare il download in attesa.

I controlli di sanità delle release sono **automatici** e non configurabili:
sottotitoli hardcoded (`HC`) e dimensioni assurde (una soglia per risoluzione
derivata da un archivio reale) vengono rifiutati. Gli scarti sono eventi
ordinari e vengono registrati a `DEBUG` con titolo, risoluzione, dimensione e
motivo, così il log `INFO` di produzione resta pulito; abilita il debug per
vederli. Anche la protezione dagli
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

### Collegare un servizio esterno

Le integrazioni non sono necessarie per scaricare e archiviare i media. Attivale
una alla volta e usa sempre il pulsante di test quando disponibile:

- **Trakt/Simkl**: completa il flusso PIN/OAuth, verifica che l'account corretto
  sia visualizzato e solo dopo abilita import watchlist o scrobbling;
- **Jellyfin/Plex**: inserisci URL raggiungibile dal demone e token con i permessi
  minimi necessari, poi prova l'aggiornamento libreria;
- **Hook**: configura prima un programma innocuo che scriva un log, verifica i
  placeholder e solo dopo collegalo a script di automazione o notifiche.

Gli hook vengono eseguiti senza shell: pipe, redirezioni e operatori come `&&`
non vengono interpretati. Se servono, inserisci un vero script eseguibile e
passagli i valori tramite i placeholder o le variabili `REXTTO_*`. Non inserire
token o password negli argomenti se il comando finisce nei log del sistema.

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

### Backup: cosa protegge e cosa no

Un backup di Rextto protegge database e configurazione. Non contiene i video,
gli archivi multimediali né lo stato completo della sessione libtorrent. Prima di
un ripristino:

1. ferma o metti in pausa i cicli automatici;
2. conserva una copia del database attuale;
3. verifica data e dimensione del backup;
4. ripristina solo su un'installazione compatibile;
5. controlla percorsi e permessi prima di riattivare i download.

Il ripristino di un database non sposta automaticamente i file multimediali. Se
la libreria è stata spostata, correggi i percorsi o esegui una scansione archivio
prima di avviare upgrade e ricerche mancanti.

### Pulizia e operazioni irreversibili

Usa prima le anteprime quando disponibili. In particolare:

- *Anteprima duplicati* mostra cosa finirebbe nel cestino;
- *Anteprima rinomina* mostra vecchio e nuovo nome;
- *Pulizia database* riguarda righe storiche, non la libreria;
- la pulizia del cestino può eliminare definitivamente i file;
- l'eliminazione di un torrent può chiedere di cancellare anche i dati.

Non confondere **cestino**, **Archivio**, **Storico download** e **sessione
torrent**: sono insiemi diversi e una pulizia in uno non implica automaticamente
la pulizia degli altri.

## 10. Salute, Log, Grafici

- **Salute** — stato processo/sistema, runtime (3 colonne), stato del servizio
  Rextto, **raggiungibilità degli indexer**, permessi percorsi, stato sorgenti,
  ultimi errori, dischi. Per Jackett il controllo usa l'endpoint Torznab `caps`;
  un problema di API key o di configurazione degli indexer viene quindi distinto
  dalla semplice raggiungibilità della macchina.
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

### Salute: interpretare lo stato delle sorgenti

Per ogni indexer la tabella distingue:

- **OK**: risposta HTTP valida e nessun errore Torznab applicativo;
- **Errore API**: il servizio risponde, ma API key o configurazione indexer non
  sono accettate;
- **Non raggiungibile**: non è arrivata una risposta entro il timeout.

Questa distinzione è importante: riavviare Rextto non risolve un errore API key,
mentre un problema di rete può richiedere di controllare DNS, container, porta o
firewall.

## 11. Notifiche

Configura Telegram, e-mail (SMTP) o un webhook (con segreto HMAC) e invia un
test. Le notifiche di completamento includono dimensione, tempo di download e
velocità media.

### Procedura di configurazione

1. salva le credenziali nella scheda **Notifiche**;
2. abilita solo gli eventi che vuoi ricevere;
3. invia il test dalla UI;
4. controlla sia la risposta del provider sia il log Rextto;
5. esegui un test di completamento solo con un download non importante.

Per un webhook verifica il segreto HMAC sul ricevente e non confondere il test
HTTP con la consegna dell'evento reale. Per SMTP controlla host, porta, TLS,
utente e mittente: un server raggiungibile può comunque rifiutare il mittente o
richiedere autenticazione diversa.

## Riferimento rapido

### Quando usare quale azione

| Obiettivo | Azione |
|---|---|
| Capire cosa esiste online | Ricerca manuale |
| Riempire episodi mancanti | Cerca mancanti / ciclo Serie |
| Scegliere una release specifica | **Perché non questa?** → Accoda |
| Far riconoscere file già presenti | Scansiona archivio |
| Cambiare il nome senza riscaricare | Anteprima rinomina |
| Correggere un servizio esterno | Salute → sorgente → Verifica |
| Eliminare file inferiori | Anteprima duplicati → Pulizia |
| Salvare configurazione e DB | Backup |

### Glossario

- **Release**: risultato trovato da feed, indexer o motore web.
- **Ciclo**: una passata automatica di ricerca e selezione.
- **Gap**: episodio mancante riconosciuto nell'archivio.
- **Placeholder**: riga temporanea che rappresenta un download non ancora
  completato.
- **Upgrade**: sostituzione di un file archiviato con una release migliore.
- **Backoff**: pausa progressiva delle richieste a una sorgente che fallisce.
- **Seed**: condivisione del torrent dopo il completamento.
- **NAS**: destinazione di rete usata per la libreria o per i percorsi configurati.

## 12. Risoluzione problemi

- **Una sorgente non risponde** — controlla *Configurazione → Sorgenti → Verifica*
  e il pannello stato sorgenti; per Jackett verifica URL base, API key e che
  almeno un indexer sia abilitato in Jackett. Gli errori Torznab vengono mostrati
  anche quando Jackett risponde HTTP 200. I siti protetti da Cloudflare
  richiedono un FlareSolverr funzionante.
- **Jackett è raggiungibile ma mostra errore API** — copia nuovamente la API key,
  controlla che almeno un indexer sia abilitato in Jackett e verifica che l'URL
  inserito in Rextto sia quello base corretto. Non aggiungere due volte il
  percorso `/api/v2.0/indexers/all/results/torznab/api`.
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
- **La release è visibile ma non viene scelta** — apri **Perché non questa?**:
  controlla prima titolo monitorato, filtri globali, sanità, qualità/lingua e
  confronto archivio. Se tutti questi passano, guarda il controllo informativo
  sulla selezione del ciclo: altri candidati, delay, spazio libero e gap filling
  possono ancora cambiare il risultato.
- **Un file esiste sul NAS ma Rextto lo considera mancante** — controlla nome
  episodio, percorso associato alla serie e permessi; poi usa *Scansiona archivio*.
  La scansione riconosce i nomi video con stagione/episodio, non file arbitrari
  che non permettono di identificare il contenuto.
- **Il download è completo ma non compare nella libreria** — guarda il log per
  spostamento, permessi e spazio; non cancellare la sorgente finché il percorso
  archiviato non è visibile nella cronologia.
- **Il backup FTP fallisce** — usa *Test FTP*: indica il passo che fallisce
  (connessione, login, percorso remoto, upload, rimozione) e lo registra nel log.
- **Log** — vedi `data/rextto.log` (rotazione a 5 MB) o il viewer nella UI.
