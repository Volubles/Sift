# Aktuell status och nästa steg

Senast uppdaterad: 2026-10-09. Detta dokument beskriver faktiska leveranser;
produkt- och arkitekturdokumenten beskriver vad appen ska bli.

## Klart

- Lokalt Git-repo på `main`, repo-hygien och Conventional Commit-arbetsflöde.
- AGENTS.md med beständiga kvalitetskrav och instruktioner för att återuppta arbetet.
- Produktkrav för bibliotek, sökning, förhandslyssning, projekt och kopiering.
- Teknisk riktning, ansvar, trådar, lagring och native integrationsgränser.
- Föreslagna prestandabudgetar och reproducerbart mätupplägg.

Ingen `Cargo.toml`, Rustimplementation, byggd app eller CI finns ännu.
Ingen remote, publicering eller vald distributionslicens finns.

## Beslut och öppna val

- Beslutat: Rust, resurssnålt lokalt kärnflöde, projekt som referenser,
  kopiering vid extern drag-and-drop och små checkpoint-commits.
- Prototypriktning: Windows först, egui/eframe, SQLite/FTS5, Symphonia, CPAL
  och en begränsad Windowsadapter. Verifiera före definitiva biblioteksversioner.
- Öppet: UI-formgivning/språk, exakt formatmatris, enhets/backendstöd,
  godkänt provbibliotek, slutliga prestandabudgetar och distribution/licens.
- Likhetssökning är senare och valfri. AI-kategorisering ingår inte i grundplanen.

## Verifiering för detta steg

Arbetet är repo- och dokumentationsgrund. UTF-8/radslut för tio filer, elva
lokala dokumentlänkar, femton Git-ignorefall och text-/binärattribut har
kontrollerats utan fel. Staged diffs granskas vid respektive checkpoint.
Git-historiken är källan för aktuella commit-hashar och branch.

Ingen appkompilering, uppspelning, native drag-and-drop eller prestandamätning
har utförts. Förväxla inte en förberedd specifikation med fungerande runtime.

## Nästa föreslagna uppgift

När implementation beställs: skapa ett minimalt Cargo-projekt med kompatibel
låst stabil toolchain. Prova stor virtualiserad fillista, waveform,
förhandslyssning och native drag till Utforskaren. Börja med disponibla fixtures,
håll ändringar i naturliga checkpoints och mät vila/latens/minne.

Det första stora tekniska valet ska avgöras av den prototypen. Börja inte med
hela biblioteksanalysen, AI eller ett stort ramverk före detta bevis.
