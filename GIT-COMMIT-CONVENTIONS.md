# Git och checkpoint-commits

## Princip

Benjamin vill ha täta, naturliga commits som visar hur projektet byggs steg för
steg. Committera när en utvecklare rimligen vill spara ett fungerande checkpoint:
en begriplig och verifierad del av arbetet. Vänta inte tills en hel version är
färdig och begär inte ett separat godkännande för varje lokal commit.

En commit ska ha ett tydligt syfte och kunna förklaras utan chattens historia.
Implementation, nödvändiga tester och relevant dokumentation hör ofta ihop.
Storleken bestäms av sammanhanget, inte ett godtyckligt fil- eller radantal.
Skapa inte commits med en trasig app eller konstgjorda commits per enskild fil.

## Format

Följ [Conventional Commits 1.0.0](https://www.conventionalcommits.org/en/v1.0.0/):

```text
<type>(<optional scope>): <short English description>

<optional explanation of why, behavior, and validation>

<optional breaking-change or issue footer>
```

Använd lowercase type/scope och ett kort, konkret engelskt subject. Scope ska
beskriva berört ansvar, exempelvis `library`, `search`, `audio`, `projects`,
`export`, `ui`, `storage` eller `repo`. Scope är valfritt där det inte hjälper.

| Type | Användning |
| --- | --- |
| `feat` | Ny användbar funktion. |
| `fix` | Korrigering av felaktigt beteende. |
| `perf` | Beteendebevarande förbättring med relevant prestandabevis. |
| `refactor` | Ändrad kodstruktur med bevarat beteende. |
| `test` | Meningsfull verifiering utan ändrad produktfunktion. |
| `docs` | Dokumentation och specifikation. |
| `build` / `ci` | Byggkedja respektive automatiserade kontroller. |
| `chore` | Repo-/verktygsunderhåll. |
| `style` | Enbart formattering, ingen beteendeändring. |
| `revert` | Avsiktligt återtagande av tidigare ändring. |

Ange en verklig inkompatibel ändring med `!` och/eller en `BREAKING CHANGE:`
footer. Beskriv migreringskonsekvensen; en databasändring är inte automatiskt
breaking om befintliga data migreras säkert. En commit är inte en release.

## Bra checkpoints

Exempel för en större uppgift; dela bara där gränsen faktiskt är användbar:

```text
feat(library): persist registered sample roots
feat(library): index changed audio metadata in bounded batches
feat(search): query indexed sample names and tags
feat(ui): browse paginated sample search results
fix(audio): discard decoded buffers after a newer selection
feat(projects): save sample references with project ordering
feat(export): copy selected samples without replacing existing files
```

Planera naturliga checkpoints före en större implementation, men anpassa dem
efter vad som lärs under arbetet. En ny persistencegrund kan vara en självständig
checkpoint även innan UI använder den om dess kontrakt och tester är kompletta.
En ändring som kräver sin anroparuppdatering hör ihop med den uppdateringen.

## Före varje commit

1. Kontrollera branch och arbetsyta; skilj egna ändringar från befintliga.
2. Kör relevanta kontroller och granska det nya beteendet.
3. Stagea explicita paths eller granskade hunks. Använd inte blind `git add .`.
4. Läs `git diff --cached`, kontrollera `git diff --cached --check` och
   `git diff --cached --name-only`. Inga privata data eller orelaterade filer.
5. Skapa committen och kontrollera vad den innehåller. Rapportera hash + subject.

Om något är oklart: granska och lösa det före commit. En statusuppdatering ska
följa relevant checkpoint, inte skapa en oändlig kedja av housekeeping-commits.
Koppla inte på dolda hooks/installationssteg som användaren behöver konfigurera
manuellt innan en vanlig commit fungerar.

## Brancher och historik

- `main` är projektets lokala bas. Kontrollera status innan en ny branch skapas.
- Större oberoende arbete använder normalt `codex/<short-purpose>` från aktuell
  avsedd bas. Återanvänd en relevant pågående branch. Små sammanhängande ändringar
  behöver inte skapa en ny branch för varje checkpoint.
- Om en remote senare finns: kontrollera/fetcha den avsedda basen innan en
  oberoende branch. Ett oväntat divergerat läge ska utredas före integration.
- Varken push, PR eller release är automatiskt godkänd av en lokal commit.
- Använd inte reset, clean, rebase, force-push, stash eller amend för att dölja
  eller flytta orelaterat/shared arbete. Vanliga nya commits är standard.
- Förfalska inte author, datum, verifiering eller historik. Behåll fungerande
  checkpoints. Rätta en publicerad historik med en ny commit.

## Vad får checkas in?

Kod, appens `Cargo.lock`, toolchain/buildkonfiguration, små rättighetsklara
testfixtures, projektdokumentation och denna Sift-guide hör till repot.
Privata ljudbibliotek, exporter, DB, logs, råmätningar, credentials, IDE-state och
build output ska förbli utanför Git. `.gitignore` är ett stöd; staged diff är
fortfarande den avgörande kontrollen.
