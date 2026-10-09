# Teknisk riktning

Dokumentet beskriver avsedd arkitektur. Ingen av modulerna finns ännu.
Rust är beslutat; biblioteksvalen nedan ska bevisas i prototypen och därefter
registreras med valda versioner, alternativ och skäl.

## Kandidater och beslutsgränser

| Del | Föreslagen kandidat | Vad ska verifieras före låsning? |
| --- | --- | --- |
| UI | egui/eframe | Resursförbrukning, stora listor, waveform, tangentbord och native drag-and-drop. |
| Lagring | SQLite med FTS5, Rustadapter exempelvis rusqlite | Transaktioner, migrering, söksemantik, låsning och verkliga query plans. |
| Avkodning | Symphonia | Faktiska codecvarianter, seek, metadata och stora filer. |
| Ljudenhet | CPAL | Windowsenheter, sample-rate-anpassning, latens, underruns och enhetsbyte. |
| Windowsintegration | Liten native adapter | Filöverföring med endast kopiering, flerval och krockbeteende i Utforskaren. |

Ett modernt verktyg ska tillföra en konkret förbättring. Välj en kompatibel,
underhållen stabil toolchain och dokumentera versionen i `rust-toolchain.toml`
när Cargo-projektet skapas. Utgå från Rust edition 2024 om kandidaterna stöder
den. Committera appens `Cargo.lock`. Nightly, stora ramverk och nya tjänster
kräver ett faktiskt behov, inte en allmän önskan att vara modern.

Native UI är utgångspunkten. Byt UI om prototypens integrations- eller
prestandabevis visar att ett annat val ger ett bättre helhetsresultat.
Skriv ett kort beslut i detta dokument eller en avgränsad ADR innan ett större
byte. Bevara kärnlogiken så att UI-valet inte äger bibliotekets regler.

## Ansvar och beroenden

| Område | Äger | Ska inte äga |
| --- | --- | --- |
| `app` | Start, inställningar, wiring och kontrollerad shutdown | Sökregler, avkodning eller SQL i event handlers. |
| `library` | Rötter, filidentitet, skanning och bibliotekstillstånd | UI eller uppspelningsenhet. |
| `search` | Frågor, filter, stabil sortering och resultatsidor | Direkt åtkomst till UI-widgets. |
| `projects` | Projekt, objektreferenser, ordning och sparregler | Mutation av biblioteksoriginal. |
| `audio` | Decode, buffering, seek, resampling och playback | Projektlagring och sökfrågor. |
| `analysis` | Waveform och versionerade akustiska egenskaper | UI-tråd eller obegränsad fullfilsladdning. |
| `storage` | SQL, migrering och transaktionsgränser | Produktbeslut och native drag-and-drop. |
| `export` | Målplan, namnkonflikter, kopiering och utfall | Radering/flytt av källfiler. |
| `platform` | Native dialoger, sökvägsbeteende och drag-and-drop | Portabla domänregler. |
| `ui` | Presentation, interaktion och synligt tillstånd | Blockerande I/O och dold affärslogik. |

Detta är ansvarsgränser, inte krav på tio crates eller tomma modulfiler.
Börja med ett Cargo-paket och sammanhängande moduler. Bryt ut en crate först
när ett faktiskt beroende, återbruk eller testbehov motiverar det. Portabel
domänlogik ska inte bero på egui, CPAL eller Windowsobjekt.

## Datamodell och filidentitet

- `LibraryRoot`: stabilt ID, rotlokator och tillgänglighet.
- `Sample`: stabilt ID, rot och relativ OS-sökväg, filrevision och metadata.
- `Project` / `ProjectItem`: egna ID:n, sample-referens, ordning och eventuell
  virtuell mapp, exportnamn och anteckning.
- Manuella taggar/favoriter: beständiga användaruppgifter med explicit ägare.
- Waveform/analys: härledd, återbyggbar information med versionsnyckel.

Behandla inte en innehållshash som den enda filidentiteten: två olika filer kan
ha samma innehåll, och en befintlig fil kan få nytt innehåll. Ett beständigt
sample-ID, filplats och innehållsversion har olika betydelse. Dokumentera hur
OS-fil-ID, relativ sökväg och valfri hash används; tvetydig omlänkning kräver
ett val. Saknade filer behåller sina referenser och manuella uppgifter.

Bevara OS-sökvägar med `Path`/`PathBuf`. Förlustkonverterad visningstext får
inte återanvändas för filoperationer. Bestäm en round-trip-säker lagring för
stödda plattformar. Windows case/Unicode-regler ska inte förenklas till en
global lowercase-sträng som filidentitet.

## Trådar och arbetsköer

UI-tråden hanterar presentation och korta kommandon. Disk-I/O, SQL, hashing,
avkodning och analys körs i arbetare med avgränsade köer, tydligt ägarskap och
kontrollerad shutdown. SQLiteanslutningar har en avsiktlig ägare; varje jobb
ska inte skapa sin egen obegränsade tråd eller anslutning.

Snabb användarinteraktion och audio decode prioriteras framför bakgrundsanalys.
Jobb har ID/generation, cancellation och definierat resultat. En gammal
sökning, waveform eller decode får inte uppdatera ett nyare val. Sammanfoga
progresshändelser och använd backpressure så att UI inte översvämmas.

Ljudcallbacken konsumerar förberedd audio från en begränsad buffert. Den får
inte göra fil-I/O, SQL, loggning, heapallokering eller vänta på lås/andra trådar.
Vid underrun produceras definierad tystnad och en räknare som avläses utanför
callbacken. Seek/enhetsbyte använder en tydlig state machine och versionerade
buffertar så att gammalt ljud inte läcker in i en ny uppspelning.

Använd synkron Rust och ett litet antal arbetare där det räcker. Lägg till en
async-runtime först när den löser ett konkret integrationsbehov. CPU-tunga
jobb blir inte billiga bara för att de ligger i en async-funktion.

## Lagring och återhämtning

Skilj användarens beständiga uppgifter från återbyggbart bibliotek/analysindex.
En logisk separation kan börja i samma databas om radering och migrering är
tydliga. Använd versionsstyrda migreringar, foreign keys och transaktioner för
flerstegsändringar. Testa rollback och bevarande av användaruppgifter.

WAL och läs-/skrivanslutningar utvärderas mot det verkliga arbetsflödet.
Säkerhetskopiera en SQLite-databas genom en metod som ger en konsekvent snapshot;
kopiera inte en aktiv `.db` och anta att ej checkpointad WAL följde med.
En nyare osupporterad schemaversion ska ge ett fel, inte nollställa databasen.

Schemat ska separera okänt metadata från noll och fel från ännu ej analyserat.
Indexera de faktiska filter/sorteringar som används och granska query plans för
stora resultat. Versionssätt serialiserade projekt och inställningar.

## Index, cache och likhet

Skanna i begränsade batcher. Hasha eller avkoda bara när operationen kräver det;
storlek och ändringstid är billiga indikationer, inte bevis på identiskt innehåll.
Filövervakning behöver overflow/reconcile-beteende och stöd för avstängning.
En frånkopplad rot får inte behandlas som att användaren raderat alla dess filer.

Waveformcache använder flernivåtoppar när det behövs för stora filer. Cachekey
innehåller filrevision, algoritmversion och relevanta parametrar. Varje cache
ska ha storleksgräns, eviction och invalidation. Ingen cache får äga enda kopian
av manuella taggar eller projekt.

Likhetssökning börjar, om den beställs, med normaliserade versionerade
akustiska egenskaper och uppmätt kvalitet. Jämför en enkel exakt sökning innan
ett approximerande vektorindex tillkommer; stora ANN-index har egen RAM- och
underhållskostnad. Det går att lägga till embeddings senare utan att göra
AI-kategorisering obligatorisk.

## Native export

Windowsfilöverföring måste ange kopiering som enda erbjudna effekt och använda
korrekta native filreferenser. Intern UI-drag är ett annat kontrakt. Prova
med flera filer, Unicode, krockar och modifierartangenter i Utforskaren.

För appens egen export: validera mål, planera konflikter och kopiera i
begränsade block till en unik temporär fil i målmappen. Publicera den färdiga
filen med verifierad plattformsspecifik semantik; en kontroll av att målet
saknas följd av en vanlig rename är inte i sig ett skydd mot race/överskrivning.
Ersättning kräver ett uttryckligt användarval. Mål får inte aliasa källan.
Resultatet är per fil, inklusive avbrutet, misslyckat och redan kopierat.

## Referenser för implementationen

Verifiera dokumentation mot låsta versioner när kod skrivs.

- [egui](https://github.com/emilk/egui)
- [SQLite FTS5](https://sqlite.org/fts5.html)
- [Symphonia](https://github.com/pdeljanov/Symphonia)
- [CPAL](https://github.com/RustAudio/cpal)
- [Windows Shell data transfer](https://learn.microsoft.com/en-us/windows/win32/shell/datascenarios)
