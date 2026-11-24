# ADR 0001: Zielarchitektur von xHoly

- **Status:** Akzeptiert (Projektregeln vom 2026-11-24)
- **Phase:** 24B
- **Ersetzt:** alle früheren Aussagen über Xbox One, Xbox 360, ARM64-Gastcode und Pi-4-Unterstützung

## Kontext

Die Dokumentation beschrieb bisher drei verschiedene Zielsysteme. Das README nannte Xbox Series S|X und Xbox One, der Whitepaper-Text nannte Xbox One (Jaguar) und Series (Zen 2) als x86-64, der XEX2-Loader verarbeitet das Executable-Format der Xbox 360 (PowerPC), und der CPU-Pfad führt AArch64-Code aus. Zusätzlich existiert ein lokaler, nicht gepushter Commit (`0c76865`), der einen PowerPC-Decoder und Xbox-360-XAM-Ordinals aus Xenia hinzufügt. Eine technische Begründung für diese Mischung gibt es in der Dokumentation nicht.

## Entscheidung

1. **Zielplattform:** Xbox Series X|S (Gen9 / Scarlett) ist die einzige aktuelle Zielplattform.
2. **Host-Plattformen:** Windows x86-64 und Linux x86-64, gleichberechtigt. Architekturentscheidungen müssen für beide Hosts begründet sein.
3. **ARM64-Hosts:** Werden vorerst nicht unterstützt. Der Code darf weiter kompilieren, aber es gibt dafür keine Tests und keine Supportzusage.
4. **Emulationsansatz:** High-Level Emulation (HLE). Plattform-APIs werden nachgebildet, nicht das gesamte Hardwaresystem.
5. **Gast-CPU:** Gast-Code ist native x86-64. Ziel ist die Ausführung nativ auf dem x86-64-Host, mit Brücken für die Xbox-ABI. Ob dafür ein Mapping-Pfad oder ein JIT verwendet wird, wird in Phase 24E entschieden.
6. **ABI und APIs:** Xbox-ABI-Signaturen, Strukturlayouts und Funktionsnummern dürfen nur aus verifizierbaren Quellen stammen. Eigene Syscall-Nummern als Ersatz für echte ABI-Werte sind nicht zulässig. Quellen, die für eine andere Konsole gelten (z. B. Xenia für die Xbox 360), sind ausgeschlossen.
7. **ScarlettX:** Dient als technische und funktionale Kompatibilitätsreferenz. Quellcode wird nicht kopiert. Vor einer inhaltlichen Übernahme ist die Lizenz zu klären, denn im lokalen ScarlettX-Verzeichnis wurde keine LICENSE-Datei gefunden.
8. **Nicht-Ziele:** Xbox 360, Xbox One, ARM64-Gastcode als Spielziel und macOS als Host.

## Konsequenzen für bestehende Komponenten

| Komponente | Einordnung | Maßnahme |
|---|---|---|
| `src/core/cpu/interpreter.rs` (AArch64) | Testharness, kein Xbox-Ziel | Bleibt als Test-Harness, klar gekennzeichnet |
| `src/core/cpu/arm64.rs` (eigenes ISA, Phase 1) | Veraltet | Entfernen oder isolieren (Phase 24C) |
| `src/core/cpu/translator.rs`, `src/jit/` (x86-64) | Teilmenge, Stub bzw. WASM-Interpreter | Kandidat für den CPU-Pfad, Entscheidung in 24E |
| `src/core/loader/xex_parser.rs`, `image_decode.rs` (XEX2) | Xbox-360-Format | Außerhalb des Zielumfangs. Ersatz: PE32+-Loader (24D) |
| `src/core/ppc/`, `src/core/kernel/xam_abi.rs` (lokaler Commit `0c76865`) | Xbox 360 (PowerPC, Xenia-Ordinals) | Außerhalb des Zielumfangs. Nicht gelöscht, Entscheidung durch den Projektinhaber offen |
| `src/core/kernel/pte.rs`, `src/gpu/swapchain.rs`, `src/gpu/shader_vertex.rs` (lokaler Commit) | Plattformbezug unklar | Prüfen, ob Teile wiederverwendbar sind (24F) |
| `src/gui/scarletx/` (lokaler Commit) | Schreibweise uneinheitlich, verwechselbar mit ScarlettX | Umbenennen und Zielbezug prüfen (24F) |
| `src/core/kernel/xam.rs` | Stub-Tabelle mit xHoly-Syscalls | Durch ABI-Tabellen mit Layout-Tests ersetzen (24F) |
| Scala-Runtime, Legacy-Shims | Xbox-360-Bezug, nicht Teil des Ziels | Entscheidung offen: entfernen oder einfrieren |
| `cpp/window_manager/` | Win32-only | Linux-Hostpfad klären |
| macOS-Targets (`aarch64-apple-darwin`) | Nicht im Hostumfang | Aus der Installationsdokumentation entfernen |

## Host-Unterschiede (Windows gegenüber Linux)

- **Grafik:** Windows verwendet D3D12. Linux verwendet Vulkan. Beide Backends liegen hinter einer gemeinsamen Schnittstelle und werden getestet.
- **ABI:** Windows-PE-Code verwendet die Win64-Aufrufkonvention (RCX, RDX, R8, R9). Linux verwendet SysV (RDI, RSI, RDX, RCX). Auf Linux braucht die Ausführung von PE-Code daher Thunks. Das ist ein offenes technisches Risiko, das in 24E untersucht wird.
- **Loader:** Der PE-Loader muss auf beiden Hosts Dateien lesen und abbilden können. ScarlettX setzt im Prozessmodus Windows-DLL-Verzeichnisse voraus. Diese Abhängigkeit darf nicht unbemerkt übernommen werden.
- **Fenster:** Die Win32-Fensterverwaltung ist nur unter Windows verfügbar. Für Linux ist ein eigener Backend-Pfad nötig.

## Erfolgskriterien

Jede Phase hat messbare Kriterien, die in `ROADMAP.md` stehen. Eine Phase gilt erst als abgeschlossen, wenn die Tests auf Windows und Linux grün sind.

## Offene Fragen

1. Soll der PowerPC-Stand aus dem lokalen Commit `0c76865` verworfen oder in einen eigenen Branch ausgelagert werden?
2. Woher stammen die Xbox-ABI-Definitionen für Gen9, und unter welcher Lizenz dürfen sie verwendet werden?
3. Soll der Scala-Teil bleiben oder entfernt werden?
4. Ist ein Linux-Host mit nativer Ausführung von PE-Code (mit ABI-Thunks) das gewünschte Ziel, oder soll Linux zunächst nur Parsing und Rendering unterstützen?