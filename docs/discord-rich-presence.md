# Discord Rich Presence

xHoly meldet seinen Zustand optional an Discord. Die Integration ist
plattformunabhängig (`src/presence/`); die Emulations- und GUI-Threads
sprechen nur das `PresenceSink`-Trait an. Alle RPC-Aufrufe laufen auf einem
eigenen Worker-Thread.

## Verwendete RPC-Bibliothek

`discord-rich-presence` **1.1.0** (MIT, cross-platform, aktualisiert 2026-01).
Sie spricht das lokale Discord-IPC-Protokoll unter Windows und Linux.
Die Abhängigkeit liegt unter `cfg(not(target_arch = "wasm32"))`.

## Application ID — Einrichtung

**Es ist keine Application ID im Repo hinterlegt.** Ohne ID wird gar keine
Verbindung aufgebaut (kein Socket, kein Handshake).

1. https://discord.com/developers/applications öffnen → *New Application*.
2. Name exakt `xHoly` vergeben (Discord zeigt ihn als „Playing xHoly").
3. Die *Application ID* (OAuth2 → General → Application ID) kopieren.
4. In `~/.config/xholy/config.toml` eintragen:

```toml
[discord]
enabled = true
application_id = "HIER_ECHTE_ID_EINTRAGEN"
show_game_name = true
show_generation = true
```

`--no-discord-rpc` deaktiviert die Integration für die laufende Sitzung,
unabhängig von der Konfiguration.

## Angezeigte Zustände

| Zustand | Details | State |
|---|---|---|
| Hauptmenü | `Chilling in Main Menu` | – |
| Laden | `Loading {game}` | – |
| Emulation | `Emulating {game}` | `Xbox One` oder `Xbox Series X|S` |
| Spielende / Emulator-Ende | zurück zum Hauptmenü / Activity gelöscht | – |

Wiederholte identische Zustände werden verworfen (Dedupe), damit ein
Frame-Loop keine RPC-Flut erzeugt. Der `try_send`-Kanal (16 Einträge)
blockiert den Emulations-Thread niemals.

## Datenschutz

Übertragen werden ausschließlich der **Anzeigename** des Spiels und die
Konsolengeneration. Lokale Pfade, Benutzernamen und persönliche Daten
werden entfernt (`DiscordSettings::display_name`). Bei `enabled = false`
oder leerer ID wird keine RPC-Verbindung geöffnet.

## Testergebnisse

`cargo test --features cpu-stub`: 398 Lib-Tests + alle Integrationstests
grün (Windows x86-64, 2026-11-24). Enthalten: Hauptmenü, Spielstart,
Spielwechsel, Spielende, Emulator-Ende, Discord nicht installiert,
Discord geschlossen, Verbindungsabbruch/Reconnect, Dedupe, Datenschutz
sowie Windows-/Linux-Konfigurationstests. `cargo clippy --all-targets
-D warnings` sauber, `cargo fmt --check` sauber.

Die Linux-Tests sind **geschrieben, aber auf diesem Windows-Host nicht
ausgeführt** worden. Für Linux muss `cargo test` auf
`x86_64-unknown-linux-gnu` separat laufen.

## Bekannte Einschränkungen

- Ohne eigene Application ID bleibt die Presence inaktiv (Absicht).
- „Playing xHoly" ist der App-Name aus dem Developer Portal, kein
  RPC-Feld; ein falscher Portal-Name lässt sich nicht überschreiben.
- Discord-Rate-Limits (5 Updates / 20 s) werden nicht aktiv gedrosselt;
  das Dedupe reduziert Updates, aber bei sehr schnellen Spielwechseln
  kann Discord einzelne Updates verwerfen.
- Kein Join/Spectate, keine Assets/Buttons, keine Lobby-Funktionen.
- Wird Discord während der Emulation geschlossen, reconnectet der Worker
  mit 3 s Backoff; das aktuelle Activity-Wird nach dem Reconnect erneut
  gesendet.
