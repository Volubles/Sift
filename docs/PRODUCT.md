# Produktkrav

## Mål och prioritering

Sift ska vara en snabb, resurssnål desktopapp för att hitta, förhandslyssna på,
organisera och exportera ljud ur stora lokala samlingar. Ljudeffekter är ett
primärt användningsfall; samples och hela musikfiler ska också fungera.
Sononym är en funktionell referens. Sift har en egen implementation och design.

Prioritera i denna ordning: bevara användarens ljud och projekt, ge ett
responsivt arbetsflöde, hålla CPU/RAM/diskarbete lågt, leverera tydliga funktioner.
Prestanda och tillförlitlighet ingår i en funktions kvalitet.

## Beslutade utgångspunkter

- Rust är huvudspråk. Appen ska fungera lokalt utan konto eller nätverk.
- Projekt samlar referenser till ljud. Samma ljud kan användas i flera projekt.
- Drag-and-drop från appen till en extern mapp ska kopiera originalfilen.
- Bibliotekets original ska inte ändras av taggning, projekt eller export.
- AI-baserad kategorisering ingår inte i den planerade grundprodukten.
- Likhetssökning är ett möjligt senare steg; kärnflödet ska fungera utan den.
- Små, sammanhängande checkpoint-commits är det normala arbetssättet.

Windows är första föreslagna målplattformen, utifrån Benjamins arbetsmiljö.
Portabel kärnlogik ska göra fler plattformar möjliga utan att lova stöd innan det
har verifierats. Publicering, licens, UI-språk och slutlig formgivning är öppna.

## Bibliotek och indexering

Användaren ska kunna registrera flera biblioteksrötter utan att importera eller
kopiera hela ljudsamlingen. Indexet ska lagra sökbar metadata och källreferenser.
Biblioteksmappar ska gå att koppla bort från Sift utan att ljuden raderas.

En första skanning ska visa verklig progress, fel och möjlighet att pausa eller
avbryta. Senare skanningar ska främst behandla ändrade filer. Oändliga
symboliska länkcykler, dubbelregistrerade rötter och överlappande rötter ska ha
ett definierat beteende. Filändringar under skanning får inte ge falsk färdigstatus.

Första formatmålet är WAV, FLAC, MP3 och AIFF. Verifiera konkreta codecvarianter,
bitdjup och metadata i prototypen. Läsbara format är inte samma sak som full
metadata- eller uppspelningssupport. Visa begripliga fel för osupporterade,
trasiga och otillgängliga filer och fortsätt med resten av biblioteket.

Metadata ska kunna innehålla namn, sökväg, längd, storlek, format, kanaler,
samplingsfrekvens, bitdjup när meningsfullt, manuella taggar och favoriter.
Härledda analysvärden ska ange analysversion och om analysen är ofullständig.
Musikrelaterad BPM/tonhöjd är senare funktioner och får ha okänt värde.

## Hitta och förhandslyssna

- Sök i namn, sökvägar och taggar; filtrera och sortera relevant metadata.
- Hantera stora resultat med stabil sortering och avgränsade dataladdningar.
- Stöd tangentbordsnavigering, flerval och tydlig markering av aktivt ljud.
- Visa en återanvändbar vågform; stöd play/pause, stopp, seek och volym.
- Snabba byten av val ska stoppa tidigare förhandslyssning och prioritera den nya.
- Långa musikfiler ska strömmas eller buffras begränsat, även vid waveformanalys.
- Ett avbrott i ljudenheten ska ge ett tydligt tillstånd och kunna återhämtas.

En sökning får inte blockeras av bibliotekets bakgrundsanalys. Ett sent svar på
en gammal sökning får inte ersätta resultatet för användarens nya sökning.
UI ska skilja mellan ingen träff, väntande sökning och sökfel.

## Projekt och sparande

Ett projekt ska kunna skapas, döpas om, öppnas och sparas. Det innehåller valda
ljud, ordning, virtuella mappar och vid behov egna exportnamn och anteckningar.
Att ta bort ett projekt eller ett projektobjekt tar bort dess referenser.

Godkända projektändringar ska sparas transaktionellt och överleva omstart.
UI ska visa väntande eller misslyckat sparande och behålla osparade ändringar
för återförsök. En cachesanering eller full omindexering får inte ta bort
projekt, favoriter eller manuella taggar.

Ett ljud som flyttats, ändrats eller saknas ska visas med korrekt status.
Användaren ska kunna återansluta en biblioteksrot eller välja en ny källa.
Tvetydiga matchningar får inte automatiskt koppla ett projekt till fel ljud.
Att göra ett projekt självständigt genom att kopiera ljud till en projektmapp
är en senare exportfunktion; vanliga projekt ska inte duplicera biblioteket.

## Kopiering, export och drag-and-drop

Användaren ska kunna dra ett eller flera valda projektljud till Utforskaren och
få kopior med oförändrat ljudinnehåll. Export till vald mapp är ett kompletterande
arbetsflöde. Biblioteksoriginalen och projektets referenser ska finnas kvar.

Export ska planera målsökvägar innan skrivning. Krockar ska erbjudas som hoppa
över, skapa unikt namn eller uttryckligen ersätta. Utgångsläget är att inte skriva
över något. Ogiltiga Windowsnamn, Unicode, långa sökvägar, behörighetsfel,
full disk och saknade källor ska hanteras begripligt.

Avbruten export ska rapportera vilka filer som redan kopierats och vilka som
återstår. Rensa enbart temporära filer som Sift själv skapade. En batch får inte
rapporteras som helt lyckad efter en delvis misslyckad kopiering. Native
drag-and-drop ska provas i Utforskaren; stöd för DAW/appar verifieras per målapp.

## Senare funktioner

- Likhetssökning genom förberäknade akustiska egenskaper; kvalitet ska bedömas
  på både ljudeffekter och musik. Presentera likhet som en rankning.
- Exakta dubbletter genom innehållshash; akustiskt nära dubbletter som ett
  separat resultat med annan säkerhet. Ingen automatisk radering.
- Rikare taggning, sparade sökningar, exportnamn och analysfilter.
- Portabel projektbeskrivning och paketexport med tydligt schema.

Semantisk sökning, molnsynk, inspelning, pluginformat, ljudredigering,
tidslinje/DAW och full Sononym-paritet är inte första leveransen. Sådan omfattning
kräver ett konkret nytt produktbeslut.

## Föreslagna leveranssteg och acceptans

| Steg | Leverans | Acceptans |
| --- | --- | --- |
| 0 | Repo, instruktioner och specifikation | Dokumenten skiljer beslut, förslag och faktisk status. |
| 1 | Teknisk prototyp | Virtuell stor lista, uppspelning, waveform och extern kopieringsdrag fungerar på Windows; CPU/RAM mäts. |
| 2 | Bibliotek och sökning | Registrera rot, skanna, söka och starta om; ändrade/trasiga filer hanteras. |
| 3 | Projekt | Referenser, ordning och mappar sparas; original bevaras; saknade källor visas. |
| 4 | Kopiering och robusthet | Flerval, namnkonflikter, avbrott och fel verifieras med källfiler oförändrade. |
| 5 | Prestanda och distribution | Representativa mätningar, återhämtningsprov och Windowspaket är verifierade. |
| 6 | Valfri likhetssökning | Mätbar nytta, begriplig rankning och acceptabel resurskostnad. |

Stegen beskriver riktning, inte tillstånd att börja alla funktioner samtidigt.
Varje steg ska delas i naturliga implementation-checkpoints när det genomförs.

## Referenser

- [Sononym](https://www.sononym.net/)
- [Sononyms projekt](https://www.sononym.net/docs/manual/projects/)
- [Sononyms likhetssökning](https://www.sononym.net/docs/manual/similarity-search/)
