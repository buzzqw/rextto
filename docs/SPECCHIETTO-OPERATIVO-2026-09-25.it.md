# Rextto — specchietto operativo delle modifiche

Questo documento riassume le modifiche operative introdotte il 25 settembre
2026, dove verificarle e come usarle.

## 1. Torrent stalled: pausa reale e retry

### Cosa succede

Un torrent è considerato stalled quando il numero di byte completati non aumenta
per il periodo configurato. La presenza di peer, da sola, non è progresso.

Rextto quindi:

1. mette in pausa il torrent anche nel client libtorrent;
2. lo esclude dagli slot di download attivi;
3. lo mantiene nella sessione, senza cancellare dati o resume state;
4. al retry lo riprende e forza un reannounce;
5. lo rimuove solo quando scade il limite finale configurato.

### Dove si vede

- **UI:** *Scarico → Sessione torrent*, stato `stalled`;
- **Log:** `DOWNLOAD STALLED`, `stalled torrent resumed and reannounced` oppure
  `DOWNLOAD FAILED — stalled`;
- **Configurazione:** *Configurazione → libtorrent*.

### Impostazioni

| Chiave | UI | Default |
|---|---|---:|
| `libtorrent_stall_after_min` | Considera stalled dopo | 60 min |
| `libtorrent_stall_retry_min` | Retry stalled | 60 min |
| `libtorrent_stall_giveup_min` | Rimozione stalled | 20160 min |

Impostare `libtorrent_stall_giveup_min=0` conserva i torrent stalled senza
rimozione automatica.

## 2. Scoring unico

Il metodo `Config::release_score()` è il riferimento comune per:

- acquisizione automatica e upgrade;
- ordinamento delle ricerche manuali;
- post-processing e confronto con file già presenti;
- record dell’archivio e dei release visti dai feed;
- ricalcolo da *Manutenzione → Ricalcola scoring*.

Il punteggio comprende i pesi configurati, il bonus dimensione e, per i film,
il bonus dei sottotitoli preferiti. I file esistenti vengono valutati con il
nome reale e la dimensione reale; il semplice ripristino di un tag sorgente
perso da un rename non viene trattato come upgrade.

### Verifica

- **UI:** *Configurazione → Punteggi* e *Manutenzione → Ricalcola scoring*;
- **API simulatore:** `POST /api/score/preview`;
- **API rescore:** `POST /api/database/rescore`;
- **Codice principale:** `src/config.rs`, con integrazione in
  `src/orchestrator.rs`, `src/postprocess.rs`, `src/cleaner.rs`,
  `src/database.rs` e `src/web.rs`.

## 3. Cartelle osservate

### Uso

In *Configurazione → Acquisizione → Cartelle osservate* configurare:

- `path`: directory da controllare;
- `enabled`: abilita/disabilita la cartella;
- `recursive`: include sottocartelle;
- `delete_after`: elimina il file dopo l’importazione; se falso lo rinomina
  `.imported`.

Il worker controlla le cartelle ogni 15 secondi, aspetta due rilevazioni
consecutive con dimensione e timestamp invariati e solo dopo prova a leggere il
file. Gli errori di importazione non vengono più abbandonati dopo cinque
tentativi: il retry continua con backoff da 15 secondi fino a un massimo di 30
minuti. Se il file cambia, la sequenza di stabilità e il backoff ripartono.

### Verifica

- **API:** `GET/POST /api/watched-folders`;
- **Log:** `watched folder: torrent added`, `retry scheduled`;
- **Codice:** `src/watcher.rs` e worker in `src/web.rs`.

## 4. Hook eventi

In *Integrazioni → Hook eventi* si può associare un programma agli eventi Rextto.
Sono disponibili placeholder come `{event}`, `{title}`, `{hash}`, `{path}`,
`{series}`, `{season}`, `{episode}`, `{quality_score}`; gli stessi valori sono
esposti come variabili `REXTTO_*`.

- **API:** `GET/POST /api/event-hooks`;
- esecuzione senza shell;
- timeout predefinito: **60 secondi**;
- timeout massimo: 24 ore;
- un valore assente o `0` usa il default di 60 secondi.

Il timeout viene applicato in `src/hooks.rs`; il dispatch è gestito da
`src/notifier.rs`.

## 5. Compatibilità configurazione film

I database precedenti alla colonna `disable_upgrades` continuano a caricare i
film configurati, usando `false` come valore predefinito. Il loader non deve
più trasformare una query incompatibile in una lista vuota.

Per controllare direttamente i dati:

```bash
sqlite3 data/rextto_config.db \
  "select count(*), sum(enabled) from movies_config;"
```

Il conteggio dei **film configurati** viene da `movies_config`; il conteggio dei
film già scaricati nella libreria viene invece da `rextto_series.db`.

## 6. Deploy e controllo rapido

```bash
cargo build --release
systemctl restart rextto.service
systemctl is-active rextto.service
curl --fail http://127.0.0.1:5000/api/status
curl --fail http://127.0.0.1:5000/api/config
```

Per controllare il commit e il working tree:

```bash
git log -1 --oneline
git status --short --branch
```

## 7. RAM disk libtorrent

In *Configurazione → libtorrent → RAM disk e porte* Rextto mostra i percorsi
`tmpfs`/`ramfs` disponibili, con spazio libero e permessi di scrittura. Scegliere
un percorso e premere **Usa questo percorso**.

Se non è configurato alcun percorso e `/dev/shm` è disponibile, premere
**Crea in /dev/shm**: Rextto crea `/dev/shm/rextto`, imposta i permessi privati,
lo salva nella configurazione e abilita il RAM disk. Quando si preme **Usa
questo percorso**, Rextto calcola e compila anche dimensione massima per torrent,
margine libero e spazio minimo in base alla capacità reale del percorso.

In *RAM disk e porte* il pulsante **Testa porte** controlla quali porte
dell'intervallo configurato sono libere per il bind locale. Una porta in uso dal
listener libtorrent è normale mentre il servizio è attivo; il test non verifica
da solo l'apertura sul router o la raggiungibilità da Internet.

> `/dev/shm` è volatile: il contenuto e la directory creata non sopravvivono al
> riavvio della macchina. Dopo il riavvio usare nuovamente il pulsante di
> creazione. Un percorso `tmpfs` già montato dal sistema non viene montato o
> modificato da Rextto.
