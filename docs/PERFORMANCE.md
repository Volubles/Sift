# Prestanda och bevis

## Status och föreslagna budgetar

Inga appmätningar finns ännu. Siffrorna nedan är föreslagna prototypbudgetar,
inte användarbekräftade SLA:er eller uppnådda resultat. De gör prioriteringen
resurssnål mätbar. Ändra dem genom ett motiverat produktbeslut efter mätning;
höj dem inte i tysthet för att få en regression att passera.

Referensscenario: Windows, lokal SSD, release-build, färdigindexerade 100 000
metadataobjekt, begränsad resultatsida, ingen aktiv skanning eller uppspelning.
Registrera faktisk hårdvara och innehåll; syntetisk metadata ersätter inte
ljudfiler vid audio- eller skanningsmätning.

| Egenskap | Föreslaget mål | Definition |
| --- | --- | --- |
| CPU i vila | Högst 0,5 % av hela maskinens CPU | Medel över 60 s efter stabilisering, alla Siftprocesser; ange logiska kärnor. |
| Minne i vila | Högst 150 MiB privata processbytes | Samma scenario; working set, GPU-minne och mapped index redovisas separat. |
| Varm start | Användbart bibliotek inom 1 s | Processstart till interaktiv lista; befintligt index, ingen återanalys. |
| Textsökning | p95 högst 100 ms | Användarinmatning till synligt korrekt resultat; debounce och render räknas. |
| Varm förhandslyssning | p95 högst 150 ms | Play/val till första ljud; redovisa cache, enhet och buffertstorlek. |
| Scrollning | p95 CPU-framarbete inom 16,7 ms vid 60 Hz | Mät aktiv scrollning och synliga rader; rapportera även tappade frames. |

Ljudlatens kan kräva loopback/extern inspelning för verkligt bevis. Tiden till
att ett block köas i CPAL är bara en intern delmätning. Ett användbart UI utan
färdig bakgrundsmetadata kan vara en korrekt start; visa status tydligt.

## Scenario som ska mätas

1. Tom app, vila, minimerad app och app med bibliotek utan aktivitet.
2. 10 000 och 100 000 metadataobjekt; 1 000 000 som senare stresstest.
3. Kall respektive varm start och kall/varm första förhandslyssning.
4. Sökning, sortering och scrollning, inklusive snabb ändring av sökfråga.
5. WAV/FLAC/MP3/AIFF: korta effekter, långa filer, olika kanaler och sample rates.
6. Uppspelning och snabba val/seek under aktiv skanning/analys.
7. Paus, cancellation, enhetsbortfall och nedstängning med jobb i kö.
8. Projekt med många referenser och export av många eller stora filer.
9. Upprepad öppna/stänga, byta projekt och omindexera för att hitta läckor.

NAS/external disk är egna scenarier och ska inte blandas med SSD-resultat.
Syntetiska fixtures ska beskrivas som sådana. Begär ett representativt
bibliotek eller använd godkända provfiler; skanna inte privata mappar av vana.

## Mätmetod

- Använd samma releaseprofil, commit, maskin och energiinställningar vid jämförelse.
- Ange OS, CPU, logiska kärnor, RAM, disk, ljudenhet, backend och sample rate.
- Registrera filantal, totalstorlek, format/längdfördelning och indexstorlek.
- Beskriv exakt vad kall/varm betyder; manipulera inte systemcache utan behov.
- Mät latens i upprepade operationer och redovisa antal, median, p95 och max.
- Mät CPU/RAM som tidsserie; en ensam Task Manager-bild är inte en baseline.
- Rapportera peak-minne, disk-I/O, köstorlekar och audio-underruns utöver snitt.
- Kontrollera att backend/UI-drivrutiner inte bara flyttar kostnaden till GPU.
- Spara reproducerbara kommandon och metod. Privata rådata hör i ignorerad
  `benchmarks/results/`; en sanerad sammanfattning kan checkas in.

Genomströmning vid skanning ska delas i enumeration, metadata, decode,
waveform, hashing och analys. Filantal per sekund utan innehållsbeskrivning
är inte jämförbart. CPU-budget för bakgrundsarbete ska kunna begränsas och
uppspelning/UI ska prioriteras. Vila och full analys har olika budgetar.

## Förhindra strukturell resursförbrukning

Inga obegränsade köer eller caches. Ingen helbiblioteksdecode i RAM. Inget
kontinuerligt repaint eller tätt pollande när appen saknar synliga ändringar.
Ladda bara relevanta metadata/resultat och rita synliga rader. Bakgrundsjobb
ska kunna pausa, avbrytas och lämna systemet i ett definierat tillstånd.

Profilera en verifierad flaskhals före optimering. Behåll en enkel baseline
som jämförelse. Mikromätningar av en algoritm bevisar inte lägre appminne,
bättre söklatens eller störningsfri uppspelning. En `perf`-commit ska redovisa
relevanta före/efter-data och bevara beteendet.

## Underlag för en benchmarkrapport

```text
Commit / buildprofil / datum:
Maskin / OS / ljudenhet:
Bibliotek / scenario / cacheläge:
Metod / kommandon / antal repetitioner:
CPU / privata bytes / working set / GPU / peak:
Latens median / p95 / max:
Disk-I/O / underruns / köer:
Jämförelse med baseline:
Begränsningar / rådataplats / slutsats:
```
