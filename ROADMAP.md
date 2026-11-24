# xHoly Roadmap

Diese Roadmap gilt ab Phase 24B. Die Zielarchitektur steht in [`docs/adr/0001-target-architecture.md`](docs/adr/0001-target-architecture.md).

**Ziel:** Xbox-Series-X|S-Titel (Gen9) über HLE auf Windows x86-64 und Linux x86-64 ausführen, die auch ScarlettX unterstützt.

**Stand:** Keine Retail-Titel laufen. Eine Phase gilt erst als abgeschlossen, wenn alle Erfolgskriterien auf Windows und Linux belegt sind.

## Bisherige Phasen (Historie)

Die Phasen 1 bis 24A sind das bisherige Scaffold. Ihre Nummern bleiben in `STATUS.md` erhalten, werden aber nicht mehr als Fortschrittsmaß verwendet. Prozentangaben wie „62 %" gelten nicht mehr, weil sie nicht aus Tests abgeleitet waren.

Der lokale, nicht gepushte Commit `0c76865` (PowerPC, PTE, Xenia-XAM-Ordinals, Swapchain, ScarlettX-GUI) gehört nicht zum Zielumfang. Er wird nicht weiterentwickelt, bis Frage 1 aus ADR 0001 entschieden ist.

## Phase 24B: Architekturentscheidung und Dokumentation

- **Inhalt:** ADR 0001, diese Roadmap, konsistente Zielangaben in README, Whitepaper, STATUS, MISSING, INSTALL, CONTRIBUTING und Cargo.toml.
- **Erfolgskriterien:**
  - Keine Dokumentation nennt Xbox One, Xbox 360 oder ARM64-Hosts als Ziel. Sie erscheinen nur noch als Nicht-Ziele.
  - `cargo test` ist unverändert grün (kein Code wurde geändert).
  - Die Phasennummerierung ist in allen Dokumenten gleich.

## Phase 24C: Hygiene

- **Inhalt:** Tests und Lints ausführen, veralteten Phase-1-Stub `arm64.rs` entfernen oder isolieren, doppelte Installationsdateien (`INSTALL.md`/`Install.md`) zusammenführen, CI-Lauf auf Windows und Linux prüfen, Statusbericht aus Testergebnissen erzeugen.
- **Erfolgskriterien:**
  - `cargo fmt --check`, `cargo clippy -- -D warnings` und `cargo test` sind auf Windows und Linux grün.
  - Ein Generator erzeugt den Statusbericht aus den Testergebnissen. Ohne explizite Feature-Liste gibt es keine Prozentangabe.
  - Keine Referenz auf `arm64.rs` mehr, außer in Historiendokumenten.

## Phase 24D: Loader und PE32+

- **Inhalt:** PE32+-Loader für Gen9-Pakete: Auswertung von `MicrosoftGame.Config`, Abbildung der Sektionen, Relokationen (DIR64/HIGHLOW), Import-Auflösung. Ein Read-only-Befehl `inspect` gibt Paketinformationen aus.
- **Erfolgskriterien:**
  - Synthetische PE32+-Testpakete (aus Quellcode gebaut, keine fremden Binärdateien) werden korrekt geparst und abgebildet.
  - `inspect` liefert auf Windows und Linux identische Ausgabe für dieselben Pakete.
  - Nicht unterstützte Formate (verschlüsseltes XVC/XVD) werden mit einem klaren Fehler abgelehnt und nicht stillschweigend behandelt.

## Phase 24E: CPU-Pfad und ABI-Brücken

- **Inhalt:** Entscheidung zwischen nativem Mapping und JIT. Umsetzung von Win64-↔-SysV-Thunks für Linux. Prüfung, ob der bestehende x86-64-Decoder verwendbar ist.
- **Erfolgskriterien:**
  - Ein synthetisches x86-64-PE-Programm liefert auf Windows und Linux dasselbe Ergebnis.
  - Thunk-Tests prüfen Argumentübergabe, Rückgabewert und Stack-Ausrichtung in beiden Richtungen.
  - Die Entscheidung für Mapping oder JIT ist in einem ADR dokumentiert.

## Phase 24F: Kernel-ABI, Speicher und GPU

- **Inhalt:** API-Tabellen mit verifizierten Signaturen und Layout-Tests, Gast-Speichermodell mit Host-Backing, Draw-Pfad vom Gast zum Host (D3D12 unter Windows, Vulkan unter Linux).
- **Erfolgskriterien:**
  - Jede ABI-Struktur hat einen Layout-Test (Größe und Offsets), dessen Quelle dokumentiert ist.
  - Ein synthetisches Dreieck wird auf beiden Hosts gezeichnet, das Ergebnis wird pixelweise verglichen.
  - Nicht verifizierte Schnittstellen sind als solche markiert und liefern einen klaren Fehler.

## Zurückgestellt

- GUI-Arbeiten (Pride-Theme, Icons, Compositor-Optimierung): pausiert, bis Phase 24E abgeschlossen ist.
- Scala-Runtime und Web-Bundle: Entscheidung offen (ADR 0001, Frage 3).
- Netzwerk-, Cloud- und Telemetrie-Backends: keine Priorität vor dem Emulationskern.