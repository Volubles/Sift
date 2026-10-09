# Sift: arbetsregler för agenter

Sift är ett lokalt, resurssnålt samplebibliotek i Rust. Huvudflödet är att hitta
ljud, förhandslyssna, spara referenser i projekt och kopiera valda ljud till
andra mappar. Skydda original och användaruppgifter. Låg CPU/RAM, responsivitet
och läsbar kod är kvalitetskrav, inte efterarbete.

## Återuppta arbetet

Vid en ny chatt eller större uppgift: läs [aktuell status](docs/STATUS.md),
kontrollera aktuell branch, `git status --short --branch` och relevant historik.
Skilj dokumenterad plan från faktisk kod. Undersök det berörda flödet och dess
anropare före ändring; utgå inte från att en tidigare chatt redan bevisat det.
För en liten textändring räcker relevant fil och berörda instruktioner.

Använd rätt dokument för uppgiften:

| När | Läs |
| --- | --- |
| Funktion, beteende eller omfattning | [Produktkrav](docs/PRODUCT.md) |
| Modulgränser, lagring, trådar eller native integration | [Arkitektur](docs/ARCHITECTURE.md) |
| Resurskostnad, stora bibliotek eller optimering | [Prestanda](docs/PERFORMANCE.md) |
| Branch, staging och checkpoint-commits | [Git-konventioner](GIT-COMMIT-CONVENTIONS.md) |

## Samarbete och omfattning

- Kommunicera med Benjamin på svenska. Kodidentifierare, testnamn och commits
  skrivs på tydlig engelska; projektdokumentation är på svenska.
- Äg undersökning, implementation och verifiering inom den beställda uppgiften.
  Fortsätt till en färdig, granskbar leverans. Be inte om ny bekräftelse för
  redan beställda lokala ändringar, relevanta tester eller checkpoint-commits.
- Förklara kort vad som påverkas innan större ändringar. Fråga bara när ett
  saknat produktbeslut eller värde faktiskt hindrar ett korrekt nästa steg.
- En beställning av dokumentation innebär att färdigställa dokumentationen;
  den ger inte automatiskt en beställning av nästa implementationsfas.
- Bevara orelaterade ändringar. Blanda inte in allmän städning eller nya
  funktioner när de inte behövs för uppgiften.
- Push, publicering, release och meddelanden till andra kräver användarens
  uttryckliga instruktion. Skapa inte en remote eller välj publik licens av vana.
- Skanna bara bibliotek som användaren angett för uppgiften eller godkända
  fixtures. Radera, flytta eller skriv inte över privata original som testmetod.
- Starta inte parallella agenter om inte användaren begär det. Vid delegation:
  avgränsa ansvar; huvudagenten äger integration, verifiering och commits.

## Produktinvarianter

- Projektobjekt är referenser. Taggning, projektändringar och export ändrar inte
  originalens namn, innehåll eller plats. Extern drag-and-drop erbjuder kopiering.
- Skilj beständiga projekt/taggar/favoriter från återbyggbart index och cache.
  Omindexering eller cachesanering får inte radera användarens arbete.
- Saknad källa, okänd metadata, analysfel och väntande jobb är olika tillstånd.
  Koppla inte en tvetydig filmatchning till ett projekt utan ett uttryckligt val.
- Appens kärnflöde fungerar offline och utan AI-kategorisering. Likhetssökning
  får inte bli ett krav för att öppna, söka eller använda vanliga projekt.
- Tyst överskrivning, delvis lyckad export presenterad som fullständigt lyckad
  och förlorade osparade projektändringar är fel även om normalflödet fungerar.

## Kodkvalitet och ägarskap

- Skriv idiomatisk, läsbar Rust. Använd beskrivande namn, raka kontrollflöden,
  sammanhängande moduler och små funktioner med en tydlig uppgift.
- Fixa grundorsaken på den minsta gräns som äger regeln. Granska alla anropare
  när delat beteende ändras. Lägg inte duplicerade specialfall i UI.
- Börja med konkreta typer. Lägg till traits/generik när de skyddar en verklig
  gräns eller ett faktiskt test-/återbruksbehov. Skapa inte tomma lager,
  universella managers, pluginramverk eller en crate per modul i förväg.
- Domänregler ska kunna testas utan UI, ljudenhet eller Windows. UI presenterar
  tillstånd; SQL, decode och filkopiering hör i sina ägande moduler.
- Håll API:n små och synlighet avsiktlig. Modellera identitet med särskilda
  ID-typer, tillstånd med enums och okända värden med `Option` där det passar.
  Undvik booleanparametrar som gör ett anrop svårt att förstå.
- Gör ägarskap och livscykler tydliga. Undvik globalt muterbart tillstånd,
  onödiga `Arc<Mutex<_>>`, dolda initkrav och clones som döljer ägarproblem.
- `unsafe` ska vara isolerat vid en nödvändig native/FFI-gräns. Dokumentera
  säkerhetsinvarianten och granska pointerlivslängd, aliasing och thread affinity.
  Använd inte unsafe för obestyrkta prestandavinster.
- Använd `Result` för felbara operationer. `unwrap`, `expect`, `panic!` och
  `unreachable!` får inte ersätta hantering av filer, användarinmatning, SQL
  eller enheter. Testkod och bevisade interna invarianter kan ha motiverade undantag.
- Ignorera inte fel med `let _ =`, tomma felgrenar eller lyckad defaultdata.
  Best-effort cleanup ska ändå redovisa misslyckande vid ägande felgräns.
- Kommentarer förklarar invarianten eller beslutet, inte vad raden redan säger.
  Tillfälliga TODO ska ha konkret återstående arbete; leverera inte central
  funktionalitet som placeholder eller `todo!()`.

## Trådar, audio och resursgränser

- Ingen disk-I/O, SQL, hashing eller lång decode/analys på UI-tråden.
- Ingen I/O, loggning, heapallokering eller blockerande synkronisering i
  ljudcallbacken. Förbered audio utanför callbacken med begränsad buffering.
- Arbetsköer, trådar, anslutningar och caches ska ha avsiktliga gränser.
  Ange ägare, invalidation, cancellation och shutdown för bakgrundsarbete.
- Prioritera användarinteraktion och uppspelning framför analys. Använd
  jobb-ID/generation så att gamla svar inte uppdaterar nytt val eller ny sökning.
- Rita bara synliga rader och uppdatera UI när det behövs. Undvik kontinuerlig
  repaint, täta timers och fullskanning i vila.
- Ladda inte bibliotekets ljud i RAM. Långa filer och stora batcher ska ha
  begränsat minne. Caches behöver storleksgräns och mätbar nytta.
- Rust eller en ny dependency är inte bevis på prestanda. Profilera verklig
  kostnad och redovisa före/efter när en ändring påstås göra appen snabbare.

## Lagring, filer och fel

- Validera alla yttre gränser: filer, metadata, sökfrågor, inställningar,
  serialiserade projekt och native API-svar. Parametrisera SQL.
- Använd versionsstyrda scheman och transaktioner för beständiga ändringar.
  Bevara data vid misslyckad migrering; en osupporterad version får inte resetta DB.
- Filidentitet, sökväg och innehållshash är olika begrepp. Bevara OS-sökvägar
  utan förlust; visningstext får inte bli lokator för en filoperation.
- Exportnamn ska hantera traversal, reserverade namn, krockar och mål som aliasar
  källan. Beakta länkar/reparse points och race där det påverkar skrivgränsen.
- Planera konflikter, använd unika temporärfiler och verifiera faktisk
  publiceringssemantik. Kopiera originalbytes; ingen omkodning utan beställning.
- Logga ett oväntat fel en gång där dess hantering ägs: operation, steg,
  relevant ID, felkedja och utfall. Visa en begriplig åtgärd i UI.
- Normal cancellation/validering är inte en felstack. Begränsa upprepade
  diagnostikmeddelanden; ingen hot-path-loggning eller privata payloads.
- Håll loggar och diagnostik lokala och begränsade. Delade rapporter ska inte
  innehålla privata biblioteksnamn, absoluta personliga sökvägar eller hemligheter.

## UI och beroenden

- Gör listor, flerval, tangentbordsfokus, playback och projektstatus tydliga.
  Visa progress, saknade filer och misslyckat sparande utan att frysa gränssnittet.
- Bevara användarens selection och relevanta vyinställningar genom uppdateringar.
  Blanda inte intern drag-and-drop med verifierat stöd för externa mål.
- Välj aktuella, stabila och underhållna verktyg efter behov. Kontrollera
  version, plattformsstöd, licens, feature flags och faktisk integrationskostnad.
- Lås toolchain och appens dependencygraf när Cargo skapas. Minimal feature-yta
  och en liten dependencygraf är utgångspunkt; uppgradera inte allt i en bugfix.
- Använd Context7 när tillgängligt för biblioteksspecifik dokumentation;
  verifiera mot låsta versioner och officiell dokumentation eller källkod.
  En docsbeskrivning är inte bevis på lokal runtimekompatibilitet.
- Inför inte en async-runtime, tjänst, nätverkskrav eller ny UI-stack utan ett
  konkret behov och ett dokumenterat beslut.

## Verifiering och färdigdefinition

Välj de kontroller som bevisar det ändrade beteendet. Relevanta tester får köras
mot disponibla fixtures inom uppgiften utan att fråga igen. För kodändringar ska
formattering, relevanta tester och lint för berörda targets passera. Behåll
verktygens exit codes; sista kommandots framgång får inte dölja ett tidigare fel.

När ett Cargo-projekt och lockfil finns är följande standardkommandon:

```powershell
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

Anpassa testurval/featurekombinationer till verklig kod och dokumenterad CI.
Kör inte dessa i ett dokumentationsrepo utan `Cargo.toml`. Bygg inte appen enbart
för branch- eller textändringar, och återkör inte dyra kontroller utan ny anledning.

- Testa domänregler, söksemantik, lagring och migrering med små riktiga fixtures.
- Audio/filer: formatvarianter, långa filer, seek, cancellation, saknad/ändrad
  källa och felaktigt innehåll. Behåll provdata små och med känd rätt att använda.
- Projekt/export: omstart, rollback, konflikter, partial failure och oförändrade
  original. Testa att omindexering bevarar manuella användaruppgifter.
- Samtidighet: stale svar, snabba val, stängning med aktiva jobb och kögränser.
  Testa kontrollerad koordinering hellre än sköra godtyckliga sleeps.
- Native/audio: verifiera verklig uppspelning och drag till Utforskaren. Mockar,
  build och en bild av UI räcker inte som bevis för dessa beteenden.
- Prestanda: följ scenarierna och rapporteringsformatet i prestandadokumentet.
- Dokumentation: kontrollera faktainnehåll, lokala länkar och staged diff;
  skriv inte implementationstester som bara speglar en låg-risk textändring.

Färdigt betyder: beställt beteende är implementerat eller dokumentationen
levererad, relevanta kontroller har rätt utfall, fel-/avbrottsfallen är hanterade,
nya ansvar och beslut är dokumenterade och en naturlig checkpoint är committad.
Om runtimebevis saknas ska exakt återstående prov och begränsning redovisas.
Påstå inte att tester, build, uppspelning eller prestanda verifierats utan bevis.

## Commits och handover

Skapa små Conventional Commits vid naturliga, verifierade checkpoints utan att
vänta på ett separat kommando från Benjamin. Ett checkpoint ska vara begripligt
och sammanhängande; samla inte en hel version i en commit och skapa inte
konstgjorda commits per fil. Stagea explicita paths och granska staged diff.
Bevara orelaterat arbete och historik. Detaljer finns i Git-konventionerna.

Uppdatera `docs/STATUS.md` vid ett avslutat större delsteg eller innan en handover,
som del av den relevanta committen. Ange faktiskt klart, bevis, öppna beslut,
eventuella hinder och nästa konkreta steg. Använd datum och beskrivningar;
undvik självrefererande commit-hashar och en extra statuscommit efter varje edit.

Slutrapporten ska kort ange resultat, relevanta filer, branch, nya commit-hashar
med subject, verifiering och eventuell kvarvarande runtimekontroll. Ange vad
Benjamin behöver göra endast när det faktiskt behövs.

AGENTS.md och projektdokumenten ska vara incheckade i Sift och hållas aktuella.
Lägg generella regler här och detaljspecifikationer i länkade dokument. Låt inte
filen växa till en upprepad chattlogg eller kopiera instruktioner från andra projekt.
